// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Merging of ordered event streams.
//!
//! [`Merge`] combines several streams, each ordered newest first, into one
//! stream ordered newest first. A heap holds the head item of every open
//! stream. Sources with a known upper time bound are opened only when the
//! newest open head is not newer than that bound, so a query that stops
//! early never opens old files. At most `max_open` sources are open at the
//! same time; once that many are open, further sources wait until an open
//! one is exhausted, which can only reorder items whose sources overlap in
//! time.
//!
//! The order of [`MergeItem::newer`] need not be transitive (journal entries
//! are compared by sequence number, monotonic time or wall-clock time,
//! depending on what two entries share). The heap only ever compares the
//! heads of streams, so the result is deterministic, although not always
//! perfectly sorted when clocks jump.

use std::cmp::Ordering;
use std::fmt;

use nurt_contracts::error::Error;
use nurt_contracts::value::Instant;

use crate::cursor::Cursor;
use crate::event::LogEvent;

/// Item of a stream that [`Merge`] can order.
pub trait MergeItem {
    /// Key that identifies copies of the same item.
    type Key: PartialEq;

    /// `Greater` when `self` is newer than `other` and comes first.
    fn newer(&self, other: &Self) -> Ordering;

    /// Time of the item, compared with the upper bounds of unopened sources.
    fn time(&self) -> Instant;

    /// Key of the item; adjacent items with equal keys are returned once.
    fn key(&self) -> Option<Self::Key>;
}

impl MergeItem for LogEvent {
    type Key = Cursor;

    fn newer(&self, other: &Self) -> Ordering {
        self.time
            .cmp(&other.time)
            .then_with(|| self.cursor.cmp(&other.cursor))
    }

    fn time(&self) -> Instant {
        self.time
    }

    fn key(&self) -> Option<Cursor> {
        Some(self.cursor.clone())
    }
}

type ItemIter<'a, T> = Box<dyn Iterator<Item = Result<T, Error>> + Send + 'a>;
type Opener<'a, T> = Box<dyn FnOnce() -> ItemIter<'a, T> + Send + 'a>;

/// Stream that [`Merge`] opens when it is needed.
pub struct Source<'a, T> {
    bound: Option<Instant>,
    open: Opener<'a, T>,
}

impl<'a, T> Source<'a, T> {
    /// Source opened before the first item is returned.
    pub fn eager<I>(items: I) -> Self
    where
        I: Iterator<Item = Result<T, Error>> + Send + 'a,
    {
        Self {
            bound: None,
            open: Box::new(move || Box::new(items)),
        }
    }

    /// Source whose items are not newer than `bound`; `open` runs when the
    /// merged stream reaches that time.
    pub fn lazy<F, I>(bound: Instant, open: F) -> Self
    where
        F: FnOnce() -> I + Send + 'a,
        I: Iterator<Item = Result<T, Error>> + Send + 'a,
    {
        Self {
            bound: Some(bound),
            open: Box::new(move || Box::new(open())),
        }
    }
}

impl<T> fmt::Debug for Source<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

struct Head<'a, T> {
    item: T,
    order: usize,
    rest: ItemIter<'a, T>,
}

/// Stream of the items of several sources, newest first.
///
/// ```
/// use nurt_contracts::value::Instant;
/// use nurt_syslog::cursor::Cursor;
/// use nurt_syslog::event::{LogEvent, StoreKind};
/// use nurt_syslog::merge::{Merge, Source};
///
/// let event = |channel: &str, nanos: i64| {
///     let cursor = Cursor::EventLog { channel: channel.to_owned(), record_id: 0 };
///     Ok(LogEvent::new(Instant::from_unix_nanos(nanos), "app", 6, "m", StoreKind::EventLog, cursor))
/// };
/// let system = vec![event("System", 30), event("System", 10)];
/// let application = vec![event("Application", 20)];
/// let merged = Merge::new(vec![Source::eager(system.into_iter()), Source::eager(application.into_iter())], 8);
/// let times: Vec<_> = merged.map(|event| event.unwrap().time.as_unix_nanos()).collect();
/// assert_eq!(times, [30, 20, 10]);
/// ```
pub struct Merge<'a, T: MergeItem> {
    pending: Vec<(usize, Source<'a, T>)>,
    heads: Vec<Head<'a, T>>,
    max_open: usize,
    last_key: Option<T::Key>,
    deferred: Option<Error>,
}

impl<'a, T: MergeItem> Merge<'a, T> {
    /// Merges `sources`, keeping at most `max_open` of them open at a time.
    ///
    /// Eager sources open first; lazy sources open in the order of their
    /// bounds, newest first.
    #[must_use]
    pub fn new(sources: Vec<Source<'a, T>>, max_open: usize) -> Self {
        let mut pending: Vec<(usize, Source<'a, T>)> = sources.into_iter().enumerate().collect();
        pending.sort_by(|(left_index, left), (right_index, right)| {
            let left_bound = left.bound.map_or(i64::MAX, Instant::as_unix_nanos);
            let right_bound = right.bound.map_or(i64::MAX, Instant::as_unix_nanos);
            left_bound
                .cmp(&right_bound)
                .then_with(|| right_index.cmp(left_index))
        });
        Self {
            pending,
            heads: Vec::new(),
            max_open: max_open.max(1),
            last_key: None,
            deferred: None,
        }
    }

    /// Number of sources not opened yet.
    #[must_use]
    pub fn unopened(&self) -> usize {
        self.pending.len()
    }

    fn open_due(&mut self) -> Result<(), Error> {
        while let Some((_, source)) = self.pending.last() {
            let due = match (self.heads.first(), source.bound) {
                (None, _) | (_, None) => true,
                (Some(head), Some(bound)) => {
                    bound >= head.item.time() && self.heads.len() < self.max_open
                }
            };
            if !due {
                break;
            }
            let Some((order, source)) = self.pending.pop() else {
                break;
            };
            let mut items = (source.open)();
            match items.next() {
                Some(Ok(item)) => self.push(Head {
                    item,
                    order,
                    rest: items,
                }),
                Some(Err(error)) => return Err(error),
                None => {}
            }
        }
        Ok(())
    }

    fn first_before(&self, left: usize, right: usize) -> bool {
        match (self.heads.get(left), self.heads.get(right)) {
            (Some(left), Some(right)) => match left.item.newer(&right.item) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => left.order < right.order,
            },
            _ => false,
        }
    }

    fn push(&mut self, head: Head<'a, T>) {
        self.heads.push(head);
        let mut index = self.heads.len() - 1;
        while index > 0 {
            let parent = (index - 1) / 2;
            if !self.first_before(index, parent) {
                break;
            }
            self.heads.swap(index, parent);
            index = parent;
        }
    }

    fn pop(&mut self) -> Option<Head<'a, T>> {
        if self.heads.is_empty() {
            return None;
        }
        let head = self.heads.swap_remove(0);
        let mut index = 0;
        loop {
            let left = 2 * index + 1;
            let right = left + 1;
            let mut first = index;
            if self.first_before(left, first) {
                first = left;
            }
            if self.first_before(right, first) {
                first = right;
            }
            if first == index {
                break;
            }
            self.heads.swap(index, first);
            index = first;
        }
        Some(head)
    }

    fn next_item(&mut self) -> Option<Result<T, Error>> {
        if let Some(error) = self.deferred.take() {
            return Some(Err(error));
        }
        if let Err(error) = self.open_due() {
            return Some(Err(error));
        }
        let Head {
            item,
            order,
            mut rest,
        } = self.pop()?;
        match rest.next() {
            Some(Ok(next)) => self.push(Head {
                item: next,
                order,
                rest,
            }),
            Some(Err(error)) => self.deferred = Some(error),
            None => {}
        }
        Some(Ok(item))
    }
}

impl<T: MergeItem> Iterator for Merge<'_, T> {
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = match self.next_item()? {
                Ok(item) => item,
                Err(error) => return Some(Err(error)),
            };
            let key = item.key();
            if key.is_some() && key == self.last_key {
                continue;
            }
            self.last_key = key;
            return Some(Ok(item));
        }
    }
}

impl<T: MergeItem> fmt::Debug for Merge<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Merge")
            .field("open", &self.heads.len())
            .field("unopened", &self.pending.len())
            .field("max_open", &self.max_open)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::error::ErrorCode;
    use proptest::prelude::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    #[derive(Debug, Clone, PartialEq)]
    struct Item {
        time: i64,
        tag: u32,
    }

    impl MergeItem for Item {
        type Key = (i64, u32);

        fn newer(&self, other: &Self) -> Ordering {
            self.time
                .cmp(&other.time)
                .then_with(|| other.tag.cmp(&self.tag))
        }

        fn time(&self) -> Instant {
            Instant::from_unix_nanos(self.time)
        }

        fn key(&self) -> Option<(i64, u32)> {
            Some((self.time, self.tag))
        }
    }

    fn items(times: &[i64], tag: u32) -> Vec<Result<Item, Error>> {
        times
            .iter()
            .map(|time| Ok(Item { time: *time, tag }))
            .collect()
    }

    fn times(merge: Merge<'_, Item>) -> Vec<i64> {
        merge.map(|item| item.unwrap().time).collect()
    }

    #[test]
    fn merges_streams_newest_first() {
        let merge = Merge::new(
            vec![
                Source::eager(items(&[9, 5, 1], 0).into_iter()),
                Source::eager(items(&[8, 6, 2], 1).into_iter()),
                Source::eager(items(&[], 2).into_iter()),
                Source::eager(items(&[7], 3).into_iter()),
            ],
            8,
        );
        assert_eq!(times(merge), [9, 8, 7, 6, 5, 2, 1]);
    }

    #[test]
    fn equal_items_from_different_sources_are_returned_once() {
        let merge = Merge::new(
            vec![
                Source::eager(items(&[5, 3], 0).into_iter()),
                Source::eager(items(&[5, 4], 0).into_iter()),
            ],
            8,
        );
        assert_eq!(times(merge), [5, 4, 3]);
    }

    #[test]
    fn lazy_source_opens_only_when_reached() {
        let opened = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&opened);
        let mut merge = Merge::new(
            vec![
                Source::eager(items(&[10, 9, 8], 0).into_iter()),
                Source::lazy(Instant::from_unix_nanos(5), move || {
                    counter.fetch_add(1, AtomicOrdering::SeqCst);
                    items(&[5, 4], 1).into_iter()
                }),
            ],
            8,
        );
        let first: Vec<_> = merge
            .by_ref()
            .take(3)
            .map(|item| item.unwrap().time)
            .collect();
        assert_eq!(first, [10, 9, 8]);
        assert_eq!(opened.load(AtomicOrdering::SeqCst), 0);
        assert_eq!(merge.unopened(), 1);
        assert_eq!(times(merge), [5, 4]);
        assert_eq!(opened.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn lazy_sources_open_in_bound_order() {
        let merge = Merge::new(
            vec![
                Source::lazy(Instant::from_unix_nanos(3), || {
                    items(&[3, 1], 0).into_iter()
                }),
                Source::lazy(Instant::from_unix_nanos(9), || {
                    items(&[9, 2], 1).into_iter()
                }),
                Source::lazy(Instant::from_unix_nanos(6), || items(&[6], 2).into_iter()),
            ],
            8,
        );
        assert_eq!(times(merge), [9, 6, 3, 2, 1]);
    }

    #[test]
    fn open_limit_defers_sources_until_one_is_exhausted() {
        let open = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let sources = (0..4)
            .map(|tag| {
                let open = Arc::clone(&open);
                let peak = Arc::clone(&peak);
                Source::lazy(Instant::from_unix_nanos(100), move || {
                    let now = open.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                    peak.fetch_max(now, AtomicOrdering::SeqCst);
                    let close = Arc::clone(&open);
                    items(&[100 - i64::from(tag)], tag)
                        .into_iter()
                        .chain(std::iter::from_fn(move || {
                            close.fetch_sub(1, AtomicOrdering::SeqCst);
                            None
                        }))
                })
            })
            .collect();
        let merge = Merge::new(sources, 2);
        assert!(format!("{merge:?}").contains("max_open: 2"));
        assert_eq!(times(merge).len(), 4);
        assert!(peak.load(AtomicOrdering::SeqCst) <= 2);
    }

    #[test]
    fn error_of_a_stream_is_returned_after_its_item() {
        let failing: Vec<Result<Item, Error>> = vec![
            Ok(Item { time: 9, tag: 0 }),
            Err(Error::new(ErrorCode::Io, "read failed")),
        ];
        let mut merge = Merge::new(vec![Source::eager(failing.into_iter())], 8);
        assert_eq!(merge.next().unwrap().unwrap().time, 9);
        assert_eq!(merge.next().unwrap().unwrap_err().code(), ErrorCode::Io);
        assert!(merge.next().is_none());
    }

    #[test]
    fn error_when_opening_a_source_is_returned() {
        let failing: Vec<Result<Item, Error>> = vec![Err(Error::new(ErrorCode::Io, "open failed"))];
        let mut merge = Merge::new(vec![Source::eager(failing.into_iter())], 8);
        assert_eq!(merge.next().unwrap().unwrap_err().code(), ErrorCode::Io);
        assert!(format!("{:?}", Source::<Item>::eager(std::iter::empty())).contains("Source"));
    }

    #[test]
    fn log_events_order_by_time_then_cursor() {
        let event = |nanos: i64, record_id: u64| {
            LogEvent::new(
                Instant::from_unix_nanos(nanos),
                "app",
                6,
                "m",
                crate::event::StoreKind::EventLog,
                Cursor::EventLog {
                    channel: "System".to_owned(),
                    record_id,
                },
            )
        };
        assert_eq!(event(2, 0).newer(&event(1, 9)), Ordering::Greater);
        assert_eq!(event(1, 2).newer(&event(1, 1)), Ordering::Greater);
        assert_eq!(event(1, 1).key(), Some(event(1, 1).cursor));
        assert_eq!(MergeItem::time(&event(4, 0)), Instant::from_unix_nanos(4));
    }

    proptest! {
        #[test]
        fn merge_of_sorted_streams_is_sorted_and_complete(
            mut streams in proptest::collection::vec(proptest::collection::vec(0_i64..1000, 0..20), 0..8),
            max_open in 1_usize..10,
        ) {
            let mut expected = Vec::new();
            let mut sources = Vec::new();
            for (tag, stream) in streams.iter_mut().enumerate() {
                stream.sort_unstable_by(|left, right| right.cmp(left));
                stream.dedup();
                let tag = u32::try_from(tag).unwrap();
                expected.extend(stream.iter().map(|time| (*time, tag)));
                let bound = stream.first().copied().unwrap_or(0);
                let owned = items(stream, tag);
                sources.push(Source::lazy(Instant::from_unix_nanos(bound), move || owned.into_iter()));
            }
            let merged: Vec<_> = Merge::new(sources, max_open)
                .map(|item| item.map(|item| (item.time, item.tag)).unwrap())
                .collect();
            let mut sorted = expected.clone();
            sorted.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
            if max_open >= streams.len() {
                prop_assert_eq!(merged, sorted);
            } else {
                let mut all = merged.clone();
                all.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
                prop_assert_eq!(all, sorted);
            }
        }

        #[test]
        fn merge_is_deterministic_with_mixed_comparisons(
            streams in proptest::collection::vec(proptest::collection::vec((0_i64..50, 0_u32..3), 0..10), 1..5),
        ) {
            let build = || {
                let sources = streams
                    .iter()
                    .map(|stream| {
                        let owned: Vec<Result<Item, Error>> = stream
                            .iter()
                            .map(|(time, tag)| Ok(Item { time: *time, tag: *tag }))
                            .collect();
                        Source::eager(owned.into_iter())
                    })
                    .collect();
                Merge::new(sources, 8).map(|item| item.unwrap()).collect::<Vec<_>>()
            };
            prop_assert_eq!(build(), build());
        }
    }
}
