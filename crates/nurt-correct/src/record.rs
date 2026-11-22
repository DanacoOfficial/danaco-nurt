// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Correction records as shown in the effect report.
//!
//! [`Correction`] carries the members of the report correction
//! ([`nurt_contracts::report::Correction`]) with typed rule, class and kind,
//! and converts to and from it. Its [`Display`](fmt::Display) form is the
//! line of the text report:
//!
//! ```text
//! C1 L01  1:34  remvoe → remove  command
//! ```

use std::fmt;
use std::str::FromStr;

use nurt_contracts::report::{self, TextPosition};

use crate::policy::Class;

/// Highest rule number within a rule family.
pub const MAX_RULE_NUMBER: u8 = 99;

/// Prefix of correction identifiers within a report (`C1`, `C2`, …).
pub const CORRECTION_ID_PREFIX: char = 'C';

/// Family of correction rules, the first letter of a rule identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RuleFamily {
    /// Lexical and naming rules (`L01`–`L24`).
    Lexical,
    /// Type and unit rules (`T01`–`T09`).
    Type,
    /// Value name rules (`V01`–`V04`).
    Value,
    /// Translation of shell habits (`B01`–`B40`).
    Bash,
    /// Translation of PowerShell habits (`P01`–`P42`).
    PowerShell,
}

impl RuleFamily {
    /// Every family, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Lexical,
        Self::Type,
        Self::Value,
        Self::Bash,
        Self::PowerShell,
    ];

    /// Letter of the family in rule identifiers.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::Lexical => 'L',
            Self::Type => 'T',
            Self::Value => 'V',
            Self::Bash => 'B',
            Self::PowerShell => 'P',
        }
    }

    fn from_letter(letter: char) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|family| family.letter() == letter)
    }
}

/// Identifier of a correction rule: a family letter and two digits (`L01`).
///
/// # Examples
///
/// ```
/// use nurt_correct::record::{RuleFamily, RuleId};
///
/// let rule: RuleId = "B15".parse().unwrap();
/// assert_eq!(rule.family(), RuleFamily::Bash);
/// assert_eq!(rule.number(), 15);
/// assert_eq!(rule.to_string(), "B15");
/// assert_eq!(RuleId::L01.to_string(), "L01");
/// assert!("X01".parse::<RuleId>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuleId {
    family: RuleFamily,
    number: u8,
}

macro_rules! rule_constants {
    ($($name:ident = $family:ident $number:literal;)*) => {
        impl RuleId {
            $(
                #[doc = concat!("Rule `", stringify!($name), "`.")]
                pub const $name: Self = Self { family: RuleFamily::$family, number: $number };
            )*
        }
    };
}

rule_constants! {
    L01 = Lexical 1;
    L02 = Lexical 2;
    L03 = Lexical 3;
    L04 = Lexical 4;
    L05 = Lexical 5;
    L06 = Lexical 6;
    T01 = Type 1;
    T02 = Type 2;
    T03 = Type 3;
    T04 = Type 4;
    T05 = Type 5;
    T06 = Type 6;
    T07 = Type 7;
    T08 = Type 8;
    T09 = Type 9;
    V01 = Value 1;
    V02 = Value 2;
    V03 = Value 3;
    V04 = Value 4;
}

impl RuleId {
    /// Builds an identifier; `None` when `number` is 0 or above
    /// [`MAX_RULE_NUMBER`].
    #[must_use]
    pub const fn new(family: RuleFamily, number: u8) -> Option<Self> {
        if number == 0 || number > MAX_RULE_NUMBER {
            None
        } else {
            Some(Self { family, number })
        }
    }

    /// Family of the rule.
    #[must_use]
    pub const fn family(self) -> RuleFamily {
        self.family
    }

    /// Number of the rule within its family.
    #[must_use]
    pub const fn number(self) -> u8 {
        self.number
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{:02}", self.family.letter(), self.number)
    }
}

/// Error for text that is not a rule identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidRuleId(pub String);

impl fmt::Display for InvalidRuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid rule identifier `{}`: use a letter of L, T, V, B, P and two digits, for example L01",
            self.0
        )
    }
}

impl std::error::Error for InvalidRuleId {}

impl FromStr for RuleId {
    type Err = InvalidRuleId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidRuleId(text.to_owned());
        if !report::Correction::is_rule_id(text) {
            return Err(invalid());
        }
        let mut chars = text.chars();
        let family = chars
            .next()
            .and_then(RuleFamily::from_letter)
            .ok_or_else(invalid)?;
        let number = chars.as_str().parse::<u8>().map_err(|_| invalid())?;
        Self::new(family, number).ok_or_else(invalid)
    }
}

/// Kind of the corrected element, shown in the last column of a correction
/// line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CorrectionKind {
    /// Command name.
    Command,
    /// Parameter name.
    Param,
    /// Enumeration value.
    Value,
    /// Record field.
    Field,
    /// Expression function.
    Function,
    /// Reference to a `let` name.
    Ref,
    /// Size literal.
    Size,
    /// Duration literal.
    Duration,
    /// Boolean literal.
    Bool,
    /// Integer literal.
    Int,
    /// Number literal.
    Number,
    /// Integer range.
    Range,
    /// Regular expression.
    Regex,
    /// Point in time.
    Instant,
    /// Path literal.
    Path,
    /// Text literal or escape.
    Text,
    /// Quotation marks.
    Quote,
    /// Parentheses, brackets or braces.
    Bracket,
    /// Pipe operator.
    Pipe,
    /// Commas, colons and equals signs between arguments.
    Separator,
    /// Order of arguments.
    Order,
    /// White space.
    Whitespace,
    /// Reserved word of the language (`let`, `and`, `not`, …).
    Keyword,
    /// Syntax of another variant.
    Syntax,
    /// Translation of a habit of another shell.
    Habit,
}

impl CorrectionKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 25] = [
        Self::Command,
        Self::Param,
        Self::Value,
        Self::Field,
        Self::Function,
        Self::Ref,
        Self::Size,
        Self::Duration,
        Self::Bool,
        Self::Int,
        Self::Number,
        Self::Range,
        Self::Regex,
        Self::Instant,
        Self::Path,
        Self::Text,
        Self::Quote,
        Self::Bracket,
        Self::Pipe,
        Self::Separator,
        Self::Order,
        Self::Whitespace,
        Self::Keyword,
        Self::Syntax,
        Self::Habit,
    ];

    /// Stable name of the kind, as written in the report.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Param => "param",
            Self::Value => "value",
            Self::Field => "field",
            Self::Function => "function",
            Self::Ref => "ref",
            Self::Size => "size",
            Self::Duration => "duration",
            Self::Bool => "bool",
            Self::Int => "int",
            Self::Number => "number",
            Self::Range => "range",
            Self::Regex => "regex",
            Self::Instant => "instant",
            Self::Path => "path",
            Self::Text => "text",
            Self::Quote => "quote",
            Self::Bracket => "bracket",
            Self::Pipe => "pipe",
            Self::Separator => "separator",
            Self::Order => "order",
            Self::Whitespace => "whitespace",
            Self::Keyword => "keyword",
            Self::Syntax => "syntax",
            Self::Habit => "habit",
        }
    }
}

impl fmt::Display for CorrectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error for text that is not a correction kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidKind(pub String);

impl fmt::Display for InvalidKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown correction kind `{}`", self.0)
    }
}

impl std::error::Error for InvalidKind {}

impl FromStr for CorrectionKind {
    type Err = InvalidKind;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text)
            .ok_or_else(|| InvalidKind(text.to_owned()))
    }
}

/// A correction applied to the program text.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::TextPosition;
/// use nurt_correct::policy::Class;
/// use nurt_correct::record::{Correction, CorrectionKind, RuleId};
///
/// let correction = Correction::new(RuleId::L01, Class::Fuzzy, "remvoe", "remove", CorrectionKind::Command)
///     .with_id(1)
///     .with_at(TextPosition { line: 1, column: 34, length: Some(6) });
/// assert_eq!(correction.to_string(), "C1 L01  1:34  remvoe → remove  command");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    /// Identifier within the report (`C1`); empty until numbered.
    pub id: String,
    /// Rule that produced the correction.
    pub rule: RuleId,
    /// Certainty class.
    pub class: Class,
    /// Position of the corrected text in the program as written.
    pub at: Option<TextPosition>,
    /// Text as written.
    pub from: String,
    /// Text after the correction.
    pub to: String,
    /// Kind of the corrected element.
    pub kind: CorrectionKind,
    /// Other interpretations, at most three, for a probable correction of a
    /// reading program.
    pub alternatives: Vec<String>,
}

impl Correction {
    /// A correction without identifier, position and alternatives.
    #[must_use]
    pub fn new(
        rule: RuleId,
        class: Class,
        from: impl Into<String>,
        to: impl Into<String>,
        kind: CorrectionKind,
    ) -> Self {
        Self {
            id: String::new(),
            rule,
            class,
            at: None,
            from: from.into(),
            to: to.into(),
            kind,
            alternatives: Vec::new(),
        }
    }

    /// Sets the identifier to `C<number>`.
    #[must_use]
    pub fn with_id(mut self, number: usize) -> Self {
        self.id = format!("{CORRECTION_ID_PREFIX}{number}");
        self
    }

    /// Sets the position of the corrected text.
    #[must_use]
    pub fn with_at(mut self, at: TextPosition) -> Self {
        self.at = Some(at);
        self
    }

    /// Sets the alternatives.
    #[must_use]
    pub fn with_alternatives(mut self, alternatives: Vec<String>) -> Self {
        self.alternatives = alternatives;
        self
    }
}

impl fmt::Display for Correction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.id.is_empty() {
            write!(f, "{} ", self.id)?;
        }
        write!(f, "{}  ", self.rule)?;
        if let Some(at) = self.at {
            write!(f, "{}:{}  ", at.line, at.column)?;
        }
        write!(f, "{} → {}  {}", self.from, self.to, self.kind)?;
        if !self.alternatives.is_empty() {
            write!(f, "  also: {}", self.alternatives.join(", "))?;
        }
        Ok(())
    }
}

impl From<Correction> for report::Correction {
    fn from(correction: Correction) -> Self {
        Self {
            id: correction.id,
            rule: correction.rule.to_string(),
            class: correction.class,
            at: correction.at,
            from: correction.from,
            to: correction.to,
            kind: Some(correction.kind.as_str().to_owned()),
            alternatives: correction.alternatives,
        }
    }
}

/// Error converting a report correction with an unknown rule or kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidCorrection {
    /// The rule identifier is invalid.
    Rule(InvalidRuleId),
    /// The kind is missing or unknown.
    Kind(InvalidKind),
}

impl fmt::Display for InvalidCorrection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule(error) => error.fmt(f),
            Self::Kind(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for InvalidCorrection {}

impl TryFrom<report::Correction> for Correction {
    type Error = InvalidCorrection;

    fn try_from(correction: report::Correction) -> Result<Self, Self::Error> {
        let rule = correction.rule.parse().map_err(InvalidCorrection::Rule)?;
        let kind = correction
            .kind
            .as_deref()
            .unwrap_or_default()
            .parse()
            .map_err(InvalidCorrection::Kind)?;
        Ok(Self {
            id: correction.id,
            rule,
            class: correction.class,
            at: correction.at,
            from: correction.from,
            to: correction.to,
            kind,
            alternatives: correction.alternatives,
        })
    }
}

/// Ordered list of the corrections of one program; numbers them `C1`, `C2`,
/// … in the order of addition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Log {
    items: Vec<Correction>,
}

impl Log {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a correction and gives it the next identifier.
    pub fn push(&mut self, correction: Correction) -> &Correction {
        let number = self.items.len() + 1;
        self.items.push(correction.with_id(number));
        let last = self.items.len() - 1;
        &self.items[last]
    }

    /// Number of corrections.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether no correction was made.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of corrections of the given class.
    #[must_use]
    pub fn count(&self, class: Class) -> usize {
        self.items.iter().filter(|item| item.class == class).count()
    }

    /// Corrections in order.
    pub fn iter(&self) -> impl Iterator<Item = &Correction> {
        self.items.iter()
    }

    /// Converts the list to the corrections of the effect report.
    #[must_use]
    pub fn into_report(self) -> Vec<report::Correction> {
        self.items
            .into_iter()
            .map(report::Correction::from)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Correction {
        Correction::new(
            RuleId::T01,
            Class::Certain,
            "\"10 MB\"",
            "10mb",
            CorrectionKind::Size,
        )
    }

    #[test]
    fn parses_and_prints_rule_identifiers() {
        for text in ["L01", "L24", "T09", "V04", "B40", "P42"] {
            let rule: RuleId = text.parse().unwrap();
            assert_eq!(rule.to_string(), text);
        }
        for text in ["L00", "L1", "L001", "l01", "X01", "", "LA1"] {
            assert!(text.parse::<RuleId>().is_err(), "{text}");
        }
        assert_eq!(RuleId::new(RuleFamily::PowerShell, 100), None);
        assert_eq!(RuleId::V04.family(), RuleFamily::Value);
    }

    #[test]
    fn rejects_invalid_rules_with_a_hint() {
        let error = "X1".parse::<RuleId>().unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("invalid rule identifier `X1`")
        );
    }

    #[test]
    fn round_trips_kinds() {
        assert_eq!(
            "keyword".parse::<CorrectionKind>(),
            Ok(CorrectionKind::Keyword)
        );
        for kind in CorrectionKind::ALL {
            assert_eq!(kind.as_str().parse::<CorrectionKind>(), Ok(kind));
            assert_eq!(kind.to_string(), kind.as_str());
        }
        assert_eq!(
            "colour".parse::<CorrectionKind>().unwrap_err().to_string(),
            "unknown correction kind `colour`"
        );
    }

    #[test]
    fn prints_report_lines() {
        let line = sample().with_id(2).with_at(TextPosition {
            line: 1,
            column: 12,
            length: Some(7),
        });
        assert_eq!(line.to_string(), "C2 T01  1:12  \"10 MB\" → 10mb  size");
        let without_position = sample().with_id(3).with_alternatives(vec!["10mib".into()]);
        assert_eq!(
            without_position.to_string(),
            "C3 T01  \"10 MB\" → 10mb  size  also: 10mib"
        );
    }

    #[test]
    fn converts_to_and_from_the_report() {
        let correction = sample().with_id(1).with_at(TextPosition {
            line: 1,
            column: 12,
            length: Some(7),
        });
        let converted = report::Correction::from(correction.clone());
        assert_eq!(converted.rule, "T01");
        assert_eq!(converted.kind.as_deref(), Some("size"));
        let json = serde_json::to_value(&converted).unwrap();
        assert_eq!(json["class"], "C");
        assert_eq!(json["at"]["length"], 7);
        assert_eq!(Correction::try_from(converted.clone()), Ok(correction));
        let mut unknown = converted.clone();
        unknown.kind = None;
        assert!(matches!(
            Correction::try_from(unknown),
            Err(InvalidCorrection::Kind(_))
        ));
        let mut bad_rule = converted;
        bad_rule.rule = "Z99".into();
        let error = Correction::try_from(bad_rule).unwrap_err();
        assert!(error.to_string().contains("Z99"));
    }

    #[test]
    fn numbers_corrections_in_order() {
        let mut log = Log::new();
        assert!(log.is_empty());
        assert_eq!(log.push(sample()).id, "C1");
        let fuzzy = Correction::new(
            RuleId::L01,
            Class::Fuzzy,
            "remvoe",
            "remove",
            CorrectionKind::Command,
        );
        assert_eq!(log.push(fuzzy).id, "C2");
        assert_eq!(log.len(), 2);
        assert_eq!(log.count(Class::Fuzzy), 1);
        assert_eq!(log.count(Class::Probable), 0);
        assert_eq!(
            log.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            ["C1", "C2"]
        );
        let report = log.into_report();
        assert_eq!(report[1].id, "C2");
        assert!(
            report
                .iter()
                .all(|item| report::Correction::is_rule_id(&item.rule))
        );
    }
}
