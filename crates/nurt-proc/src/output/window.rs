// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lines kept for one stream: the first and the last lines, the cutting of
//! long lines and the shared byte budget of a response.

use std::collections::VecDeque;

/// Characters of one line a response keeps; a longer line is cut and marked
/// with `… [+N characters]`.
pub(super) const MAX_LINE_CHARS: usize = 400;

/// Line standing for the lines left out between the head and the tail.
const OMITTED_MARK: &str = "...";

/// Line separating groups of selected lines that are not adjacent.
const GROUP_SEPARATOR: &str = "--";

/// Bytes the omitted-lines marker takes with its line feed.
const OMITTED_MARK_BYTES: usize = OMITTED_MARK.len() + 1;

/// Bytes a group separator takes with its line feed.
const GROUP_SEPARATOR_BYTES: usize = GROUP_SEPARATOR.len() + 1;

/// Least bytes worth keeping of a line that is cut to fit the budget.
const MIN_CLIP_BYTES: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShownLine {
    /// Position among the lines shown, from 0.
    pub(super) index: u64,
    pub(super) text: String,
    /// First line of the file this line stands for, from 1.
    pub(super) raw_start: u64,
    /// Lines of the file this line stands for (more than 1 when collapsed).
    pub(super) raw_len: u64,
}

/// First and last lines of a sequence of unknown length.
#[derive(Debug)]
pub(super) struct HeadTail {
    head_limit: usize,
    tail_limit: usize,
    head: Vec<ShownLine>,
    tail: VecDeque<ShownLine>,
    total: u64,
}

impl HeadTail {
    pub(super) fn new(head_limit: usize, tail_limit: usize) -> Self {
        Self {
            head_limit,
            tail_limit,
            head: Vec::new(),
            tail: VecDeque::new(),
            total: 0,
        }
    }

    pub(super) fn push(&mut self, line: ShownLine) {
        self.total += 1;
        if self.head.len() < self.head_limit {
            self.head.push(line);
        } else if self.tail_limit > 0 {
            if self.tail.len() == self.tail_limit {
                self.tail.pop_front();
            }
            self.tail.push_back(line);
        }
    }

    pub(super) fn finish(self, grouped: bool) -> Window {
        Window {
            head: self.head,
            tail: self.tail.into(),
            total: self.total,
            grouped,
            ..Window::default()
        }
    }
}

/// Lines kept for one stream.
#[derive(Debug, Default)]
pub(super) struct Window {
    pub(super) head: Vec<ShownLine>,
    pub(super) tail: Vec<ShownLine>,
    pub(super) total: u64,
    /// The lines are a selection; gaps between them are marked.
    pub(super) grouped: bool,
    /// Lines of the file the stream has.
    pub(super) raw_total: u64,
    /// Kept lines that were cut to the line limit or the byte budget.
    pub(super) cut_lines: u64,
    /// First line of the file among the cut ones.
    pub(super) first_cut: Option<u64>,
}

impl Window {
    pub(super) fn omitted(&self) -> u64 {
        self.total - (self.head.len() + self.tail.len()) as u64
    }

    /// Cuts the kept lines longer than [`MAX_LINE_CHARS`].
    pub(super) fn cut_long_lines(&mut self) {
        let mut cut = 0;
        let mut first = self.first_cut;
        for line in self.head.iter_mut().chain(self.tail.iter_mut()) {
            if let Some(shorter) = cut_line(&line.text) {
                line.text = shorter;
                cut += 1;
                first = first.or(Some(line.raw_start));
            }
        }
        self.cut_lines += cut;
        self.first_cut = first;
    }

    /// Keeps the last `count` of the kept lines and drops the head, as the
    /// error list of a recognised tool replaces the beginning of the output.
    pub(super) fn keep_last(&mut self, count: usize) {
        let mut all: Vec<ShownLine> = std::mem::take(&mut self.head);
        all.append(&mut self.tail);
        let skip = all.len().saturating_sub(count);
        self.tail = all.split_off(skip);
    }

    /// Text of the kept lines without blank lines at the start and the end;
    /// lines left out between the head and the tail are marked.
    pub(super) fn text(&self) -> String {
        let mut items: Vec<Option<&ShownLine>> = self.head.iter().map(Some).collect();
        if self.omitted() > 0 && !self.head.is_empty() && !self.tail.is_empty() {
            items.push(None);
        }
        items.extend(self.tail.iter().map(Some));
        let blank = |item: &Option<&ShownLine>| item.is_some_and(|line| line.text.is_empty());
        let start = items
            .iter()
            .position(|item| !blank(item))
            .unwrap_or(items.len());
        let end = items
            .iter()
            .rposition(|item| !blank(item))
            .map_or(start, |last| last + 1);
        let mut parts: Vec<&str> = Vec::new();
        let mut previous: Option<u64> = None;
        for item in &items[start..end] {
            match item {
                None => {
                    parts.push(OMITTED_MARK);
                    previous = None;
                }
                Some(line) => {
                    if self.grouped && previous.is_some_and(|index| line.index > index + 1) {
                        parts.push(GROUP_SEPARATOR);
                    }
                    parts.push(&line.text);
                    previous = Some(line.index);
                }
            }
        }
        parts.join("\n")
    }

    /// Range of lines of the file between the kept head and the kept tail
    /// when lines were left out; `None` for a selection of lines, whose gaps
    /// are not one range.
    pub(super) fn unseen_range(&self) -> Option<(u64, u64)> {
        if self.omitted() == 0 || self.raw_total == 0 || self.grouped {
            return None;
        }
        let from = self
            .head
            .last()
            .map_or(1, |line| line.raw_start + line.raw_len);
        let to = self
            .tail
            .first()
            .map_or(self.raw_total, |line| line.raw_start - 1);
        (from <= to).then_some((from, to))
    }
}

/// `text` cut to [`MAX_LINE_CHARS`] characters with the number of characters
/// left out; `None` when the line is short enough.
pub(super) fn cut_line(text: &str) -> Option<String> {
    let (end, _) = text.char_indices().nth(MAX_LINE_CHARS)?;
    let left = text[end..].chars().count();
    Some(format!("{}\u{2026} [+{left} characters]", &text[..end]))
}

/// `text` cut to at most `limit` bytes, marked like a cut line.
fn clip_to_bytes(text: &str, limit: usize) -> String {
    const MARK_RESERVE: usize = 24;
    let keep = limit.saturating_sub(MARK_RESERVE);
    let mut end = keep.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let left = text[end..].chars().count();
    format!("{}\u{2026} [+{left} characters]", &text[..end])
}

fn separator_bytes(grouped: bool, earlier: u64, later: u64) -> usize {
    if grouped && later > earlier + 1 {
        GROUP_SEPARATOR_BYTES
    } else {
        0
    }
}

/// The lines of the streams and what the budget lets each keep.
struct Share {
    /// The first lines followed by the last lines of each stream.
    lines: Vec<Vec<ShownLine>>,
    /// Position in `lines` where the lines left out lay, per stream.
    gaps: Vec<Option<usize>>,
    grouped: Vec<bool>,
    end_keep: Vec<usize>,
    start_keep: Vec<usize>,
    /// Bytes the last kept line is clipped to, per stream.
    clipped: Vec<Option<usize>>,
    left: usize,
}

impl Share {
    fn new(windows: &mut [Window], budget: usize) -> Self {
        let mut lines = Vec::new();
        let mut gaps = Vec::new();
        for window in windows.iter_mut() {
            gaps.push((window.omitted() > 0).then_some(window.head.len()));
            let mut all = std::mem::take(&mut window.head);
            all.append(&mut window.tail);
            lines.push(all);
        }
        Self {
            lines,
            gaps,
            grouped: windows.iter().map(|window| window.grouped).collect(),
            end_keep: vec![0; windows.len()],
            start_keep: vec![0; windows.len()],
            clipped: vec![None; windows.len()],
            left: budget,
        }
    }

    /// Keeps the last lines of stream `at` that fit; when not even one does,
    /// the last line is clipped to what is left.
    fn keep_end(&mut self, at: usize) {
        let lines = &self.lines[at];
        let floor = self.gaps[at].unwrap_or(0);
        let mut kept = 0;
        for position in (floor..lines.len()).rev() {
            let separator = if kept > 0 {
                separator_bytes(
                    self.grouped[at],
                    lines[position].index,
                    lines[position + 1].index,
                )
            } else {
                0
            };
            let cost = lines[position].text.len() + 1 + separator;
            if cost > self.left {
                break;
            }
            self.left -= cost;
            kept += 1;
        }
        if kept == 0 && lines.len() > floor && self.left >= MIN_CLIP_BYTES {
            self.clipped[at] = Some(self.left - 1);
            self.left = 0;
            kept = 1;
        }
        self.end_keep[at] = kept;
    }

    /// Keeps the first lines of stream `at` that fit before its last lines.
    fn keep_start(&mut self, at: usize) {
        let lines = &self.lines[at];
        let ceiling = self.gaps[at]
            .unwrap_or(lines.len())
            .min(lines.len() - self.end_keep[at]);
        let mut reserve = if self.end_keep[at] > 0 {
            OMITTED_MARK_BYTES
        } else {
            0
        };
        let mut kept = 0;
        for position in 0..ceiling {
            let separator = if position > 0 {
                separator_bytes(
                    self.grouped[at],
                    lines[position - 1].index,
                    lines[position].index,
                )
            } else {
                0
            };
            let cost = lines[position].text.len() + 1 + separator + reserve;
            if cost > self.left {
                break;
            }
            self.left -= cost;
            reserve = 0;
            kept += 1;
        }
        self.start_keep[at] = kept;
    }

    /// Puts the lines kept of stream `at` back into its window.
    fn restore(&mut self, at: usize, window: &mut Window) {
        let lines = std::mem::take(&mut self.lines[at]);
        let everything = self.start_keep[at] + self.end_keep[at] >= lines.len();
        let split = if everything {
            self.gaps[at].unwrap_or(lines.len())
        } else {
            self.start_keep[at]
        };
        let mut head = lines;
        let mut tail = head.split_off(split);
        if !everything {
            head.truncate(self.start_keep[at]);
            let drop = tail.len() - self.end_keep[at];
            tail.drain(..drop);
        }
        if let (Some(limit), Some(last)) = (self.clipped[at], tail.last_mut().or(head.last_mut())) {
            last.text = clip_to_bytes(&last.text, limit);
            window.cut_lines += 1;
            window.first_cut.get_or_insert(last.raw_start);
        }
        window.head = head;
        window.tail = tail;
    }
}

/// Shares `budget` bytes among the windows: the last lines of the streams in
/// the order of `tails` come first, then the first lines of the streams in
/// the order of `heads`. A window whose text fits is left unchanged.
pub(super) fn allocate(windows: &mut [Window], budget: usize, tails: &[usize], heads: &[usize]) {
    let total: usize = windows.iter().map(|window| window.text().len()).sum();
    if total <= budget {
        return;
    }
    let mut share = Share::new(windows, budget);
    for &at in tails {
        share.keep_end(at);
    }
    for &at in heads {
        share.keep_start(at);
    }
    for (at, window) in windows.iter_mut().enumerate() {
        share.restore(at, window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(index: u64, text: &str) -> ShownLine {
        ShownLine {
            index,
            text: text.to_owned(),
            raw_start: index + 1,
            raw_len: 1,
        }
    }

    fn window(texts: &[&str], head: usize) -> Window {
        let lines: Vec<ShownLine> = texts
            .iter()
            .enumerate()
            .map(|(index, text)| line(index as u64, text))
            .collect();
        let mut window = Window {
            total: lines.len() as u64,
            raw_total: lines.len() as u64,
            ..Window::default()
        };
        window.head = lines[..head].to_vec();
        window.tail = lines[head..].to_vec();
        window
    }

    #[test]
    fn cuts_long_lines_and_counts_the_characters_left() {
        let text = "a".repeat(MAX_LINE_CHARS + 25);
        let cut = cut_line(&text).unwrap();
        assert!(cut.starts_with(&"a".repeat(MAX_LINE_CHARS)));
        assert!(cut.ends_with("\u{2026} [+25 characters]"));
        assert_eq!(cut_line(&"b".repeat(MAX_LINE_CHARS)), None);
        let wide = "\u{105}".repeat(MAX_LINE_CHARS + 1);
        assert!(cut_line(&wide).unwrap().ends_with("[+1 characters]"));
    }

    #[test]
    fn a_window_within_the_budget_is_unchanged() {
        let mut windows = [window(&["aa", "bb", "cc"], 1)];
        allocate(&mut windows, 100, &[0], &[0]);
        assert_eq!(windows[0].text(), "aa\nbb\ncc");
        assert_eq!(windows[0].head.len(), 1);
    }

    #[test]
    fn the_end_wins_over_the_beginning() {
        let texts = ["11111", "22222", "33333", "44444", "55555"];
        let mut windows = [window(&texts, 5)];
        allocate(&mut windows, 14, &[0], &[0]);
        assert_eq!(windows[0].text(), "44444\n55555");
        assert_eq!(windows[0].omitted(), 3);
    }

    #[test]
    fn leftover_budget_goes_to_the_beginning_with_a_marker() {
        let mut windows = [window(&["11", "22", "55", "66"], 2)];
        windows[0].total = 6;
        windows[0].tail[0].index = 4;
        windows[0].tail[1].index = 5;
        allocate(&mut windows, 14, &[0], &[0]);
        assert_eq!(windows[0].text(), "11\n...\n55\n66");
    }

    #[test]
    fn stderr_is_served_before_stdout_and_both_stay_in_the_budget() {
        let mut windows = [
            window(&["out1", "out2", "out3"], 3),
            window(&["err1", "err2", "err3"], 3),
        ];
        allocate(&mut windows, 15, &[1, 0], &[0, 1]);
        let used: usize = windows.iter().map(|window| window.text().len()).sum();
        assert!(used <= 15);
        assert_eq!(windows[1].text(), "err1\nerr2\nerr3");
        assert!(windows[0].text().len() < "out1\nout2\nout3".len());
    }

    #[test]
    fn a_last_line_longer_than_the_budget_is_clipped() {
        let long = "x".repeat(300);
        let mut windows = [window(&["a", &long], 2)];
        allocate(&mut windows, 120, &[0], &[0]);
        let text = windows[0].text();
        assert!(text.len() <= 120, "{}", text.len());
        assert!(text.contains("characters]"));
        assert_eq!(windows[0].cut_lines, 1);
    }

    #[test]
    fn unseen_range_names_the_lines_of_the_file_left_out() {
        let texts = ["a", "b", "c", "d", "e", "f"];
        let mut windows = [window(&texts, 2)];
        windows[0].head.truncate(2);
        windows[0].tail = windows[0].tail.split_off(2);
        windows[0].total = 6;
        assert_eq!(windows[0].unseen_range(), Some((3, 4)));
    }

    #[test]
    fn keep_last_drops_the_head() {
        let mut window = window(&["a", "b", "c", "d"], 2);
        window.keep_last(3);
        assert_eq!(window.text(), "b\nc\nd");
        assert!(window.head.is_empty());
    }
}
