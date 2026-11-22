// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Ranking of program cards and guides for a query.
//!
//! The base ranking gives each query word points: 100 when it equals the
//! name, 40 when the name starts with it, 20 when the name contains it,
//! plus 8 for the situations of use, 5 for the summary, 5 for the category
//! and 3 for the guide description; 15 more when every word occurs in some
//! field. A program whose `not_for` task contains every word loses 30
//! points (at least 1 remains) and names its replacement. Standalone guides
//! get 30 points per word in the name and 4 per word in the description,
//! only without a category filter. Results are ordered by points, then
//! provider priority, then name.
//!
//! Outside the compatibility mode the ranking also reads aliases, tags,
//! category titles and package data, and retries words without any match
//! through the synonym table and prefix stems at half weight.

use crate::index::{Card, IndexGuide, ProgramIndex, SearchFields, fold_all};
use crate::model::{EntryKind, SectionKind};
use crate::source::SourceKind;
use crate::text::{edit_distance, fold, query_words, similar_distance, stem};

/// Points for a word equal to the name.
pub const NAME_EQUAL: u32 = 100;
/// Points for a name that starts with the word.
pub const NAME_PREFIX: u32 = 40;
/// Points for a name that contains the word.
pub const NAME_CONTAINS: u32 = 20;
/// Points for the word in the situations of use.
pub const WHEN_POINTS: u32 = 8;
/// Points for the word in the summary.
pub const PURPOSE_POINTS: u32 = 5;
/// Points for the word in the category.
pub const CATEGORY_POINTS: u32 = 5;
/// Points for the word in the guide description.
pub const GUIDE_POINTS: u32 = 3;
/// Points when every word occurs in some field.
pub const ALL_WORDS_BONUS: u32 = 15;
/// Penalty of a program not meant for the task.
pub const NOT_FOR_PENALTY: u32 = 30;
/// Points of a standalone guide per word in its name.
pub const GUIDE_NAME_POINTS: u32 = 30;
/// Points of a standalone guide per word in its description.
pub const GUIDE_DESCRIPTION_POINTS: u32 = 4;
/// Points for the word in tags and keywords.
pub const TAG_POINTS: u32 = 5;
/// Points for the word in the package description.
pub const PACKAGE_DESCRIPTION_POINTS: u32 = 2;
/// Points for a package name that contains the word.
pub const PACKAGE_NAME_POINTS: u32 = 20;
/// Divisor of points found by fallback matching.
pub const FALLBACK_DIVISOR: u32 = 2;
/// Hits read before a kind filter is applied.
pub const KIND_POOL: usize = 60;
/// Most similar names offered for an unknown name.
pub const SIMILAR_LIMIT: usize = 8;
/// Most sections returned for a query inside a guide.
pub const QUERY_SECTIONS: usize = 3;
/// Weight of a word in a section heading.
pub const HEADING_WEIGHT: u32 = 3;

/// Ranking options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    /// Retry words without a match through synonyms and stems.
    pub fallback: bool,
    /// Rank aliases, tags, category titles and package data.
    pub extra_fields: bool,
}

impl SearchOptions {
    /// The base ranking only, identical to the reference algorithm.
    pub const COMPATIBLE: Self = Self {
        fallback: false,
        extra_fields: false,
    };
    /// Ranking of the commands.
    pub const FULL: Self = Self {
        fallback: true,
        extra_fields: true,
    };
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self::FULL
    }
}

/// Kinds of results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KindFilter {
    /// Programs, applications and guides.
    #[default]
    All,
    /// Programs only.
    Programs,
    /// Applications only.
    Apps,
    /// Standalone guides only.
    Guides,
}

impl KindFilter {
    /// Parses `all`, `programs`, `apps` or `guides`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "all" => Some(Self::All),
            "programs" => Some(Self::Programs),
            "apps" => Some(Self::Apps),
            "guides" => Some(Self::Guides),
            _ => None,
        }
    }
}

/// Providers a result may come from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceFilter {
    /// Provider kinds.
    pub kinds: Vec<SourceKind>,
    /// Provider ids, including `catalog:<id>`.
    pub ids: Vec<String>,
}

impl SourceFilter {
    /// Returns `true` when the filter admits everything.
    #[must_use]
    pub fn is_all(&self) -> bool {
        self.kinds.is_empty() && self.ids.is_empty()
    }

    /// Returns `true` when the card has a matching provider.
    #[must_use]
    pub fn admits(&self, card: &Card, index: &ProgramIndex) -> bool {
        self.is_all()
            || card.sources.iter().any(|id| {
                self.ids.iter().any(|wanted| wanted == id.as_str())
                    || index
                        .source(id)
                        .is_some_and(|source| self.kinds.contains(&source.kind()))
            })
    }

    /// Returns `true` when a guide of provider `id` passes the filter.
    #[must_use]
    pub fn admits_guide(&self, guide: &IndexGuide) -> bool {
        self.is_all()
            || self.kinds.contains(&SourceKind::Catalog)
            || self
                .ids
                .iter()
                .any(|wanted| wanted == guide.source.as_str())
    }
}

/// One search.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    /// Query text.
    pub text: &'a str,
    /// Category id; results of other categories are left out.
    pub category: Option<&'a str>,
    /// Kinds of results.
    pub kind: KindFilter,
    /// Providers.
    pub sources: &'a SourceFilter,
    /// Installed programs only (`true`), catalog entries only (`false`).
    pub installed: Option<bool>,
    /// Most results.
    pub limit: usize,
}

/// Target of a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Card at a position of the index.
    Card(usize),
    /// Standalone guide at a position of the index.
    Guide(usize),
}

/// One result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Target.
    pub target: Target,
    /// Points.
    pub points: u32,
    /// Replacement program when the query describes a task the program is
    /// not meant for.
    pub instead: Option<String>,
}

/// Searches the index.
#[must_use]
pub fn search(index: &ProgramIndex, query: &Query<'_>, options: SearchOptions) -> Vec<Hit> {
    let words = query_words(query.text, options.extra_fields);
    let category = query.category.map(fold);
    let mut hits: Vec<(Hit, usize, &str)> = Vec::new();
    for (position, card) in index.cards().iter().enumerate() {
        if !card_admitted(card, index, query, category.as_deref()) {
            continue;
        }
        let points = card_points(index, &card.fields, &words, options);
        if points == 0 {
            continue;
        }
        let instead = substitute(card, &words);
        let points = if instead.is_some() {
            points.saturating_sub(NOT_FOR_PENALTY).max(1)
        } else {
            points
        };
        hits.push((
            Hit {
                target: Target::Card(position),
                points,
                instead,
            },
            card.rank,
            &card.name,
        ));
    }
    if category.is_none() && matches!(query.kind, KindFilter::All | KindFilter::Guides) {
        hits.extend(guide_hits(index, &words, query.sources));
    }
    hits.sort_by(|left, right| {
        right
            .0
            .points
            .cmp(&left.0.points)
            .then(left.1.cmp(&right.1))
            .then(left.2.cmp(right.2))
    });
    let filtered = hits
        .into_iter()
        .map(|(hit, _, _)| hit)
        .filter(|hit| kind_matches(index, hit, query.kind));
    if query.kind == KindFilter::All {
        filtered.take(query.limit).collect()
    } else {
        filtered.take(KIND_POOL).take(query.limit).collect()
    }
}

fn kind_matches(index: &ProgramIndex, hit: &Hit, kind: KindFilter) -> bool {
    match (kind, hit.target) {
        (KindFilter::All, _) => true,
        (KindFilter::Guides, target) => matches!(target, Target::Guide(_)),
        (KindFilter::Programs, Target::Card(position)) => index
            .card(position)
            .is_some_and(|card| card.kind == EntryKind::Program),
        (KindFilter::Apps, Target::Card(position)) => index
            .card(position)
            .is_some_and(|card| card.kind == EntryKind::App),
        _ => false,
    }
}

fn card_admitted(
    card: &Card,
    index: &ProgramIndex,
    query: &Query<'_>,
    category: Option<&str>,
) -> bool {
    if let Some(category) = category {
        let sub = card.subcategory.as_deref().map(fold);
        if card.fields.category != category && sub.as_deref() != Some(category) {
            return false;
        }
    }
    if query
        .installed
        .is_some_and(|installed| installed != card.installed)
    {
        return false;
    }
    query.sources.admits(card, index)
}

fn word_points(fields: &SearchFields, word: &str, extra: bool) -> u32 {
    let mut points = if fields.name == word {
        NAME_EQUAL
    } else if fields.name.starts_with(word) {
        NAME_PREFIX
    } else if fields.name.contains(word) {
        NAME_CONTAINS
    } else if extra && fields.aliases.iter().any(|alias| alias == word) {
        NAME_PREFIX
    } else {
        0
    };
    let has = |field: &str, weight: u32| if field.contains(word) { weight } else { 0 };
    points += has(&fields.when, WHEN_POINTS)
        + has(&fields.purpose, PURPOSE_POINTS)
        + has(&fields.category, CATEGORY_POINTS)
        + has(&fields.guide, GUIDE_POINTS);
    if extra {
        points += has(&fields.tags, TAG_POINTS)
            + has(&fields.package_description, PACKAGE_DESCRIPTION_POINTS)
            + has(&fields.package, PACKAGE_NAME_POINTS);
        if !fields.category.contains(word) {
            points += has(&fields.category_title, CATEGORY_POINTS);
        }
    }
    points
}

fn card_points(
    index: &ProgramIndex,
    fields: &SearchFields,
    words: &[String],
    options: SearchOptions,
) -> u32 {
    let mut points = 0;
    let mut everywhere = true;
    for word in words {
        let direct = word_points(fields, word, options.extra_fields);
        if direct > 0 {
            points += direct;
            continue;
        }
        let fallback = if options.fallback {
            fallback_points(index, fields, word, options.extra_fields)
        } else {
            0
        };
        if fallback == 0 {
            everywhere = false;
        }
        points += fallback;
    }
    if !words.is_empty() && everywhere && points > 0 {
        points + ALL_WORDS_BONUS
    } else {
        points
    }
}

/// Best points of the synonyms and the stem of a word, halved; at least 1
/// when any of them matches.
fn fallback_points(index: &ProgramIndex, fields: &SearchFields, word: &str, extra: bool) -> u32 {
    let stemmed = stem(word);
    let mut candidates: Vec<&str> = index.synonyms().alternatives(word);
    if let Some(stem) = stemmed.as_deref() {
        candidates.push(stem);
    }
    candidates
        .into_iter()
        .map(|candidate| word_points(fields, candidate, extra))
        .max()
        .filter(|best| *best > 0)
        .map_or(0, |best| (best / FALLBACK_DIVISOR).max(1))
}

/// Replacement program when every query word occurs in a `not_for` task
/// and the query does not name the program itself.
#[must_use]
pub fn substitute(card: &Card, words: &[String]) -> Option<String> {
    if words.is_empty() || words.contains(&card.fields.name) {
        return None;
    }
    card.not_for.iter().find_map(|item| {
        let instead = item.instead.as_deref()?.trim().trim_end_matches('.');
        let task = fold_all(&item.task);
        (!instead.is_empty() && words.iter().all(|word| task.contains(word.as_str())))
            .then(|| instead.to_owned())
    })
}

fn guide_hits<'a>(
    index: &'a ProgramIndex,
    words: &[String],
    sources: &SourceFilter,
) -> Vec<(Hit, usize, &'a str)> {
    let mut hits = Vec::new();
    for (position, guide) in index.guides().iter().enumerate() {
        let name = guide.guide.name.as_str();
        if index.guide_is_linked(name)
            || index.resolve(name).is_some()
            || !sources.admits_guide(guide)
        {
            continue;
        }
        let points: u32 = words
            .iter()
            .map(|word| {
                let in_name = if guide.folded_name.contains(word.as_str()) {
                    GUIDE_NAME_POINTS
                } else {
                    0
                };
                let in_summary = if guide.folded_summary.contains(word.as_str()) {
                    GUIDE_DESCRIPTION_POINTS
                } else {
                    0
                };
                in_name + in_summary
            })
            .sum();
        if points > 0 {
            hits.push((
                Hit {
                    target: Target::Guide(position),
                    points,
                    instead: None,
                },
                guide.rank,
                name,
            ));
        }
    }
    hits
}

/// Names similar to an unknown name: names of search hits, then names and
/// aliases within an edit distance of a third of the length, closest first.
#[must_use]
pub fn similar(index: &ProgramIndex, requested: &str, limit: usize) -> Vec<String> {
    let everything = SourceFilter::default();
    let query = Query {
        text: requested,
        category: None,
        kind: KindFilter::All,
        sources: &everything,
        installed: None,
        limit,
    };
    let mut names: Vec<String> = search(index, &query, SearchOptions::COMPATIBLE)
        .into_iter()
        .filter_map(|hit| hit_name(index, &hit).map(str::to_owned))
        .collect();
    let wanted = fold(requested);
    let allowed = similar_distance(wanted.chars().count());
    let mut near: Vec<(usize, &str)> = index
        .cards()
        .iter()
        .map(|card| card.name.as_str())
        .chain(index.alias_names().map(|(name, _)| name))
        .map(|name| (edit_distance(&wanted, &fold(name)), name))
        .filter(|(distance, _)| *distance <= allowed)
        .collect();
    near.sort();
    for (_, name) in near {
        if !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    names.truncate(limit);
    names
}

/// Name of the target of a hit.
#[must_use]
pub fn hit_name<'a>(index: &'a ProgramIndex, hit: &Hit) -> Option<&'a str> {
    match hit.target {
        Target::Card(position) => index.card(position).map(|card| card.name.as_str()),
        Target::Guide(position) => index
            .guides()
            .get(position)
            .map(|guide| guide.guide.name.as_str()),
    }
}

/// Part of a guide or of a card description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading; empty for the text before the first heading.
    pub heading: String,
    /// Text including the heading line.
    pub body: String,
}

/// Sections of a Markdown guide at `## ` headings outside code blocks.
#[must_use]
pub fn guide_sections(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut fenced = false;
    for line in text.split('\n') {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && line.starts_with("## ") {
            sections.push(Section {
                heading: line[3..].trim().to_owned(),
                body: line.to_owned(),
            });
        } else if let Some(last) = sections.last_mut() {
            last.body.push('\n');
            last.body.push_str(line);
        } else {
            sections.push(Section {
                heading: String::new(),
                body: line.to_owned(),
            });
        }
    }
    sections
}

/// Labelled description parts of a card as sections.
#[must_use]
pub fn card_sections(card: &Card, languages: &[String]) -> Vec<Section> {
    card.sections
        .iter()
        .filter_map(|section| {
            let heading = section
                .label
                .clone()
                .unwrap_or_else(|| section_label(section.kind).to_owned());
            let text = section.text.pick(languages).0;
            (!heading.is_empty()).then(|| Section {
                body: format!("{heading}: {text}"),
                heading,
            })
        })
        .collect()
}

/// English label of a section kind.
#[must_use]
pub fn section_label(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Purpose => "Purpose",
        SectionKind::When => "When to use",
        SectionKind::WhenNot => "When not to use",
        SectionKind::Io => "Input and output",
        SectionKind::Example => "Invocation",
        SectionKind::Check => "Check",
        SectionKind::Pitfalls => "Pitfalls",
        SectionKind::Related => "Related programs",
        SectionKind::Aliases => "Aliases",
        SectionKind::Other => "",
    }
}

/// Section named `wanted`: equal heading after folding, otherwise the first
/// heading that contains it; leading `#` signs are ignored.
#[must_use]
pub fn find_section<'a>(sections: &'a [Section], wanted: &str) -> Option<&'a Section> {
    let key = fold(wanted.trim_start_matches('#').trim());
    sections
        .iter()
        .find(|section| fold(&section.heading) == key)
        .or_else(|| {
            sections.iter().find(|section| {
                !section.heading.is_empty() && fold(&section.heading).contains(&key)
            })
        })
}

/// Up to [`QUERY_SECTIONS`] sections matching the words of `query`; words
/// in the heading count [`HEADING_WEIGHT`] times.
#[must_use]
pub fn match_sections<'a>(sections: &'a [Section], query: &str) -> Vec<&'a Section> {
    let words = query_words(query, false);
    let mut scored: Vec<(u32, usize, &Section)> = sections
        .iter()
        .enumerate()
        .map(|(position, section)| {
            let heading = fold(&section.heading);
            let body = fold(&section.body);
            let points = words
                .iter()
                .map(|word| {
                    let in_heading = if heading.contains(word.as_str()) {
                        HEADING_WEIGHT
                    } else {
                        0
                    };
                    in_heading + u32::from(body.contains(word.as_str()))
                })
                .sum();
            (points, position, section)
        })
        .filter(|(points, _, _)| *points > 0)
        .collect();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    scored
        .into_iter()
        .take(QUERY_SECTIONS)
        .map(|(_, _, section)| section)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_split_outside_code_blocks() {
        let text = "intro\n## Use\nrun it\n```\n## not a heading\n```\n## Pitfalls\nbeware";
        let sections = guide_sections(text);
        let headings: Vec<&str> = sections
            .iter()
            .map(|section| section.heading.as_str())
            .collect();
        assert_eq!(headings, vec!["", "Use", "Pitfalls"]);
        assert!(sections[1].body.contains("## not a heading"));
        assert_eq!(
            find_section(&sections, "## pitfalls").map(|s| s.heading.as_str()),
            Some("Pitfalls")
        );
        assert_eq!(
            find_section(&sections, "pit").map(|s| s.heading.as_str()),
            Some("Pitfalls")
        );
        assert!(find_section(&sections, "missing").is_none());
        let matched = match_sections(&sections, "beware use");
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].heading, "Use");
    }

    #[test]
    fn kind_filter_values() {
        assert_eq!(KindFilter::parse("guides"), Some(KindFilter::Guides));
        assert_eq!(KindFilter::parse("skills"), None);
        assert_eq!(section_label(SectionKind::Pitfalls), "Pitfalls");
    }
}
