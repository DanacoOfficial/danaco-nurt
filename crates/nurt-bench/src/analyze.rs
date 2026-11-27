// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bench::analyze
// Responsibility:   Analysis of run logs: result tables, paired tests and the entry rule.
// Dependencies:     serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Analysis of run logs: result tables, paired tests and the entry rule.
//!
//! Runs with outcome `error` are excluded and counted. A Danaco Nurt run is
//! paired with the shell run (bash or PowerShell) of the same model, task
//! and seed. Completion is compared with the exact McNemar test, costs of
//! pairs completed in both conditions with the Wilcoxon signed-rank test,
//! and every rate and difference carries a percentile bootstrap interval.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::condition::Condition;
use crate::error::{BenchError, Result};
use crate::metrics::{NurtCounts, RunMeasures};
use crate::runlog::{self, Outcome};
use crate::stats::{Bootstrap, Interval, mcnemar_exact, median, wilcoxon_signed_rank};
use crate::task::{HostOs, Platform};

/// Significance level of the protocol.
pub const ALPHA: f64 = 0.05;

/// Label of the rows that pool every model.
pub const ALL_MODELS: &str = "all";

/// Domain of the provocation tasks.
pub const PROVOCATION_DOMAIN: u8 = 12;

/// Rate with its bootstrap interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rate {
    /// Point estimate.
    pub value: f64,
    /// Bootstrap interval.
    pub interval: Option<Interval>,
}

/// Results of one model under one condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionRow {
    /// Model label or [`ALL_MODELS`].
    pub model: String,
    /// Condition.
    pub condition: Condition,
    /// Runs.
    pub runs: usize,
    /// Share of completed runs.
    pub completed: Rate,
    /// Share of completed runs without harm.
    pub passed: Rate,
    /// Share of runs with harm.
    pub harmed: Rate,
    /// Lost entries in all runs.
    pub lost: usize,
    /// Files with a changed encoding or line ending style in all runs.
    pub encoding: usize,
    /// Median of input and output tokens of the completed runs.
    pub median_tokens: f64,
    /// Median of model calls of the completed runs.
    pub median_steps: f64,
    /// Median of the wall clock time of the completed runs in seconds.
    pub median_seconds: f64,
    /// Median of the description tokens, when counted.
    pub median_description_tokens: Option<f64>,
    /// Share of runs with an intermediate script.
    pub script: f64,
    /// Danaco Nurt calls.
    pub nurt: Option<NurtRates>,
}

/// Measures of the Danaco Nurt calls.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NurtRates {
    /// Calls.
    pub calls: u32,
    /// Share written correctly without correction (FV).
    pub fv: f64,
    /// Share run thanks to a correction (FC).
    pub fc: f64,
    /// FV + FC (FE).
    pub fe: f64,
    /// Share of false corrections among corrected calls (FCR).
    pub fcr: f64,
    /// Share of `E_AMBIGUOUS` (AMB).
    pub amb: f64,
}

/// Paired comparison of a Danaco Nurt condition with the shell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// Model label or [`ALL_MODELS`].
    pub model: String,
    /// Danaco Nurt condition.
    pub condition: Condition,
    /// Pairs.
    pub pairs: usize,
    /// Completion rate of the shell on the pairs.
    pub shell_completed: f64,
    /// Completion rate of Danaco Nurt on the pairs.
    pub nurt_completed: f64,
    /// Difference Danaco Nurt − shell with its interval.
    pub difference: Rate,
    /// Pairs completed only with the shell.
    pub shell_only: u64,
    /// Pairs completed only with Danaco Nurt.
    pub nurt_only: u64,
    /// p-value of the exact McNemar test.
    pub mcnemar_p: f64,
    /// Pairs completed in both conditions.
    pub both_completed: usize,
    /// Cost measures on the pairs completed in both conditions.
    pub costs: Vec<CostComparison>,
    /// Completion criterion: the completion rate of Danaco Nurt is at least that of the shell.
    pub completion_met: bool,
    /// Cost criterion: every cost median of Danaco Nurt is at most that of the shell.
    pub cost_met: bool,
}

/// Paired comparison of one cost measure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostComparison {
    /// `tokens`, `steps` or `seconds`.
    pub measure: String,
    /// Median of the shell.
    pub shell_median: f64,
    /// Median of Danaco Nurt.
    pub nurt_median: f64,
    /// Difference of the medians with its interval.
    pub difference: Rate,
    /// p-value of the Wilcoxon signed-rank test.
    pub wilcoxon_p: f64,
}

/// Results of one domain under one condition, pooled over the models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainRow {
    /// Domain.
    pub domain: u8,
    /// Condition.
    pub condition: Condition,
    /// Runs.
    pub runs: usize,
    /// Share of completed runs.
    pub completed: f64,
    /// Share of runs with harm.
    pub harmed: f64,
}

/// Results of the provocation tasks of one class under one condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvocationRow {
    /// Shell failure class.
    pub provocation: String,
    /// Condition.
    pub condition: Condition,
    /// Runs.
    pub runs: usize,
    /// Share of runs with harm.
    pub harmed: f64,
    /// Share of completed runs.
    pub completed: f64,
}

/// Command corrections on cross-system tasks under one condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossRow {
    /// Condition.
    pub condition: Condition,
    /// Runs of cross-system tasks.
    pub runs: usize,
    /// Mean number of tool calls that follow a failed call.
    pub mean_retries: f64,
    /// Share of runs with at least one such call.
    pub with_retries: f64,
}

/// Entry rule of the protocol: no harm in every run, and the completion and
/// cost criteria pooled and in
/// at least two thirds of the models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryRule {
    /// Danaco Nurt condition the rule is evaluated for.
    pub condition: Condition,
    /// No Danaco Nurt run caused harm.
    pub no_harm: bool,
    /// Completion criterion pooled over the models.
    pub completion_pooled: bool,
    /// Cost criterion pooled over the models.
    pub cost_pooled: bool,
    /// Models meeting the completion and cost criteria.
    pub models_meeting: usize,
    /// Models compared.
    pub models: usize,
    /// Models required (4 of 6, generally two thirds rounded up).
    pub models_required: usize,
    /// Whether the rule holds.
    pub satisfied: bool,
}

/// Complete analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// Format identifier, `nurt-bench-results/1`.
    pub format: String,
    /// Runs analysed.
    pub runs: usize,
    /// Runs excluded because of outcome `error`.
    pub excluded_errors: usize,
    /// Bootstrap resamples.
    pub resamples: usize,
    /// Bootstrap seed.
    pub seed: u64,
    /// Rows per model and condition.
    pub conditions: Vec<ConditionRow>,
    /// Paired comparisons.
    pub comparisons: Vec<Comparison>,
    /// Rows per domain and condition.
    pub domains: Vec<DomainRow>,
    /// Rows of the provocation tasks.
    pub provocations: Vec<ProvocationRow>,
    /// Rows of the cross-system tasks.
    pub cross: Vec<CrossRow>,
    /// Entry rule per Danaco Nurt condition.
    pub entry: Vec<EntryRule>,
}

fn share(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 / total as f64
    }
}

/// Number of the cluster of every task name, in the order of the first
/// appearance: the runs of one task form one cluster of the bootstrap.
fn cluster_ids<'a>(tasks: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    tasks
        .into_iter()
        .map(|task| {
            let next = seen.len();
            *seen.entry(task).or_insert(next)
        })
        .collect()
}

fn rate(outcomes: &[bool], clusters: &[usize], bootstrap: &Bootstrap) -> Rate {
    Rate {
        value: share(
            outcomes.iter().filter(|value| **value).count(),
            outcomes.len(),
        ),
        interval: bootstrap.proportion_by_cluster(outcomes, clusters),
    }
}

fn nurt_rates(runs: &[&RunMeasures]) -> Option<NurtRates> {
    let total = runs.iter().fold(NurtCounts::default(), |mut total, run| {
        total.calls += run.nurt.calls;
        total.valid += run.nurt.valid;
        total.corrected += run.nurt.corrected;
        total.false_corrections += run.nurt.false_corrections;
        total.ambiguous += run.nurt.ambiguous;
        total
    });
    if total.calls == 0 {
        return None;
    }
    let calls = f64::from(total.calls);
    Some(NurtRates {
        calls: total.calls,
        fv: f64::from(total.valid) / calls,
        fc: f64::from(total.corrected) / calls,
        fe: f64::from(total.valid + total.corrected) / calls,
        fcr: if total.corrected == 0 {
            0.0
        } else {
            f64::from(total.false_corrections) / f64::from(total.corrected)
        },
        amb: f64::from(total.ambiguous) / calls,
    })
}

fn condition_row(
    model: &str,
    condition: Condition,
    runs: &[&RunMeasures],
    bootstrap: &Bootstrap,
) -> ConditionRow {
    let completed: Vec<&&RunMeasures> = runs.iter().filter(|run| run.completed).collect();
    let flags =
        |pick: fn(&RunMeasures) -> bool| runs.iter().map(|run| pick(run)).collect::<Vec<bool>>();
    let descriptions: Vec<f64> = runs
        .iter()
        .filter_map(|run| run.description_tokens)
        .map(|t| t as f64)
        .collect();
    let clusters = cluster_ids(runs.iter().map(|run| run.task.as_str()));
    ConditionRow {
        model: model.to_owned(),
        condition,
        runs: runs.len(),
        completed: rate(&flags(|run| run.completed), &clusters, bootstrap),
        passed: rate(&flags(|run| run.passed), &clusters, bootstrap),
        harmed: rate(&flags(|run| run.harmed), &clusters, bootstrap),
        lost: runs.iter().map(|run| run.lost).sum(),
        encoding: runs.iter().map(|run| run.encoding).sum(),
        median_tokens: median(
            &completed
                .iter()
                .filter(|run| run.tokens_measured)
                .map(|run| run.tokens() as f64)
                .collect::<Vec<_>>(),
        ),
        median_steps: median(
            &completed
                .iter()
                .map(|run| f64::from(run.steps))
                .collect::<Vec<_>>(),
        ),
        median_seconds: median(
            &completed
                .iter()
                .map(|run| run.wall_ms as f64 / 1000.0)
                .collect::<Vec<_>>(),
        ),
        median_description_tokens: (!descriptions.is_empty()).then(|| median(&descriptions)),
        script: share(runs.iter().filter(|run| run.script).count(), runs.len()),
        nurt: if condition.is_nurt() {
            nurt_rates(runs)
        } else {
            None
        },
    }
}

/// Model, task, seed and operating system of a run.
type PairKey<'a> = (&'a str, &'a str, u64, &'static str);

fn pair_key(run: &RunMeasures) -> PairKey<'_> {
    (
        run.model.as_str(),
        run.task.as_str(),
        run.seed,
        run.host.display_name(),
    )
}

/// The shell run to which a Danaco Nurt run of the same model, task, seed
/// and operating system is compared. When both shells ran there, the shell
/// of the system is taken: PowerShell on Windows, bash elsewhere.
fn shell_index(runs: &[RunMeasures]) -> BTreeMap<PairKey<'_>, &RunMeasures> {
    let mut index = BTreeMap::new();
    for run in runs.iter().filter(|run| !run.condition.is_nurt()) {
        let native = match run.host {
            HostOs::Windows => Condition::Pwsh,
            HostOs::Linux | HostOs::Macos => Condition::Bash,
        };
        let key = pair_key(run);
        if run.condition == native || !index.contains_key(&key) {
            index.insert(key, run);
        }
    }
    index
}

fn cost(
    measure: &str,
    shell: &[f64],
    nurt: &[f64],
    clusters: &[usize],
    bootstrap: &Bootstrap,
) -> CostComparison {
    let shell_median = median(shell);
    let nurt_median = median(nurt);
    CostComparison {
        measure: measure.to_owned(),
        shell_median,
        nurt_median,
        difference: Rate {
            value: nurt_median - shell_median,
            interval: bootstrap.paired_median_difference_by_cluster(nurt, shell, clusters),
        },
        wilcoxon_p: wilcoxon_signed_rank(nurt, shell).p_value,
    }
}

fn compare(
    model: &str,
    condition: Condition,
    pairs: &[(&RunMeasures, &RunMeasures)],
    bootstrap: &Bootstrap,
) -> Comparison {
    let shell: Vec<bool> = pairs.iter().map(|(shell, _)| shell.completed).collect();
    let nurt: Vec<bool> = pairs.iter().map(|(_, nurt)| nurt.completed).collect();
    let shell_only = pairs
        .iter()
        .filter(|(s, n)| s.completed && !n.completed)
        .count() as u64;
    let nurt_only = pairs
        .iter()
        .filter(|(s, n)| !s.completed && n.completed)
        .count() as u64;
    let both: Vec<_> = pairs
        .iter()
        .filter(|(s, n)| s.completed && n.completed)
        .collect();
    let series = |pick: fn(&RunMeasures) -> f64| -> (Vec<f64>, Vec<f64>) {
        both.iter().map(|(s, n)| (pick(s), pick(n))).unzip()
    };
    // Runs whose provider reported no token counts are not measured; they
    // are left out of the token comparison and not counted as 0.
    let measured: Vec<_> = both
        .iter()
        .filter(|(s, n)| s.tokens_measured && n.tokens_measured)
        .collect();
    let (shell_tokens, nurt_tokens): (Vec<f64>, Vec<f64>) = measured
        .iter()
        .map(|(s, n)| (s.tokens() as f64, n.tokens() as f64))
        .unzip();
    let (shell_steps, nurt_steps) = series(|run| f64::from(run.steps));
    let (shell_seconds, nurt_seconds) = series(|run| run.wall_ms as f64 / 1000.0);
    let pair_clusters = cluster_ids(pairs.iter().map(|(_, n)| n.task.as_str()));
    let both_clusters = cluster_ids(both.iter().map(|(_, n)| n.task.as_str()));
    let measured_clusters = cluster_ids(measured.iter().map(|(_, n)| n.task.as_str()));
    let costs = vec![
        cost(
            "tokens",
            &shell_tokens,
            &nurt_tokens,
            &measured_clusters,
            bootstrap,
        ),
        cost(
            "steps",
            &shell_steps,
            &nurt_steps,
            &both_clusters,
            bootstrap,
        ),
        cost(
            "seconds",
            &shell_seconds,
            &nurt_seconds,
            &both_clusters,
            bootstrap,
        ),
    ];
    let shell_completed = share(shell.iter().filter(|v| **v).count(), pairs.len());
    let nurt_completed = share(nurt.iter().filter(|v| **v).count(), pairs.len());
    let cost_met = !both.is_empty()
        && costs
            .iter()
            .all(|cost| cost.nurt_median <= cost.shell_median);
    Comparison {
        model: model.to_owned(),
        condition,
        pairs: pairs.len(),
        shell_completed,
        nurt_completed,
        difference: Rate {
            value: nurt_completed - shell_completed,
            interval: bootstrap.paired_proportion_difference_by_cluster(
                &nurt,
                &shell,
                &pair_clusters,
            ),
        },
        shell_only,
        nurt_only,
        mcnemar_p: mcnemar_exact(shell_only, nurt_only),
        both_completed: both.len(),
        costs,
        completion_met: !pairs.is_empty() && nurt_completed >= shell_completed,
        cost_met,
    }
}

fn comparisons(
    runs: &[RunMeasures],
    models: &BTreeSet<&str>,
    bootstrap: &Bootstrap,
) -> Vec<Comparison> {
    let shells = shell_index(runs);
    let mut result = Vec::new();
    for condition in [Condition::Nurt, Condition::NurtGroups] {
        let pairs: Vec<(&RunMeasures, &RunMeasures)> = runs
            .iter()
            .filter(|run| run.condition == condition)
            .filter_map(|run| shells.get(&pair_key(run)).map(|shell| (*shell, run)))
            .collect();
        if pairs.is_empty() {
            continue;
        }
        for model in models {
            let own: Vec<_> = pairs
                .iter()
                .copied()
                .filter(|(_, nurt)| nurt.model == *model)
                .collect();
            if !own.is_empty() {
                result.push(compare(model, condition, &own, bootstrap));
            }
        }
        result.push(compare(ALL_MODELS, condition, &pairs, bootstrap));
    }
    result
}

fn entry_rules(runs: &[RunMeasures], comparisons: &[Comparison]) -> Vec<EntryRule> {
    [Condition::Nurt, Condition::NurtGroups]
        .into_iter()
        .filter_map(|condition| {
            let rows: Vec<&Comparison> = comparisons
                .iter()
                .filter(|c| c.condition == condition)
                .collect();
            let pooled = rows.iter().find(|c| c.model == ALL_MODELS)?;
            let models: Vec<&&Comparison> = rows.iter().filter(|c| c.model != ALL_MODELS).collect();
            let meeting = models
                .iter()
                .filter(|c| c.completion_met && c.cost_met)
                .count();
            let required = (2 * models.len()).div_ceil(3);
            let no_harm = runs
                .iter()
                .filter(|run| run.condition == condition)
                .all(|run| !run.harmed);
            Some(EntryRule {
                condition,
                no_harm,
                completion_pooled: pooled.completion_met,
                cost_pooled: pooled.cost_met,
                models_meeting: meeting,
                models: models.len(),
                models_required: required,
                satisfied: no_harm
                    && pooled.completion_met
                    && pooled.cost_met
                    && meeting >= required,
            })
        })
        .collect()
}

fn grouped<K: Ord + Clone>(
    runs: &[RunMeasures],
    key: impl Fn(&RunMeasures) -> Option<K>,
) -> BTreeMap<K, Vec<&RunMeasures>> {
    let mut groups: BTreeMap<K, Vec<&RunMeasures>> = BTreeMap::new();
    for run in runs {
        if let Some(key) = key(run) {
            groups.entry(key).or_default().push(run);
        }
    }
    groups
}

fn rates_of(runs: &[&RunMeasures]) -> (f64, f64) {
    (
        share(runs.iter().filter(|run| run.completed).count(), runs.len()),
        share(runs.iter().filter(|run| run.harmed).count(), runs.len()),
    )
}

/// Analyses the measures of complete runs.
#[must_use]
pub fn analyse(all: &[RunMeasures], bootstrap: &Bootstrap) -> Analysis {
    let runs: Vec<RunMeasures> = all
        .iter()
        .filter(|run| run.outcome != Outcome::Error)
        .cloned()
        .collect();
    let models: BTreeSet<&str> = runs.iter().map(|run| run.model.as_str()).collect();
    let mut conditions = Vec::new();
    for ((model, condition), group) in
        grouped(&runs, |run| Some((run.model.clone(), run.condition)))
    {
        conditions.push(condition_row(&model, condition, &group, bootstrap));
    }
    for (condition, group) in grouped(&runs, |run| Some(run.condition)) {
        conditions.push(condition_row(ALL_MODELS, condition, &group, bootstrap));
    }
    let domains = grouped(&runs, |run| Some((run.domain, run.condition)))
        .into_iter()
        .map(|((domain, condition), group)| {
            let (completed, harmed) = rates_of(&group);
            DomainRow {
                domain,
                condition,
                runs: group.len(),
                completed,
                harmed,
            }
        })
        .collect();
    let provocations = grouped(&runs, |run| {
        (run.domain == PROVOCATION_DOMAIN)
            .then(|| (run.provocation.clone().unwrap_or_default(), run.condition))
    })
    .into_iter()
    .map(|((provocation, condition), group)| {
        let (completed, harmed) = rates_of(&group);
        ProvocationRow {
            provocation,
            condition,
            runs: group.len(),
            harmed,
            completed,
        }
    })
    .collect();
    let cross = grouped(&runs, |run| {
        (run.platform == Platform::Cross).then_some(run.condition)
    })
    .into_iter()
    .map(|(condition, group)| CrossRow {
        condition,
        runs: group.len(),
        mean_retries: group.iter().map(|run| f64::from(run.retries)).sum::<f64>()
            / group.len() as f64,
        with_retries: share(
            group.iter().filter(|run| run.retries > 0).count(),
            group.len(),
        ),
    })
    .collect();
    let comparisons = comparisons(&runs, &models, bootstrap);
    let entry = entry_rules(&runs, &comparisons);
    Analysis {
        format: "nurt-bench-results/1".to_owned(),
        runs: runs.len(),
        excluded_errors: all.len() - runs.len(),
        resamples: bootstrap.resamples,
        seed: bootstrap.seed,
        conditions,
        comparisons,
        domains,
        provocations,
        cross,
        entry,
    }
}

/// Reads the measures of every complete log below a directory.
///
/// # Errors
///
/// Returns an error when a directory or a log cannot be read.
pub fn load_measures(root: &Path, include_mock: bool) -> Result<Vec<RunMeasures>> {
    let mut measures = Vec::new();
    for path in runlog::find_logs(root)? {
        let events = runlog::read(&path)?;
        if let Some(run) = RunMeasures::from_events(&events)
            && (include_mock || !run.mock)
        {
            measures.push(run);
        }
    }
    Ok(measures)
}

fn percent(value: f64) -> String {
    format!("{:.1}", value * 100.0)
}

fn percent_rate(rate: &Rate) -> String {
    match rate.interval {
        Some(interval) => format!(
            "{} [{}, {}]",
            percent(rate.value),
            percent(interval.low),
            percent(interval.high)
        ),
        None => percent(rate.value),
    }
}

fn number(value: f64) -> String {
    if value.is_nan() {
        "-".to_owned()
    } else {
        format!("{value:.1}")
    }
}

fn p_value(value: f64) -> String {
    if value < 0.001 {
        "<0.001".to_owned()
    } else {
        format!("{value:.3}")
    }
}

fn yes(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Renders the analysis as Markdown tables.
#[must_use]
pub fn render_markdown(analysis: &Analysis) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Results\n\nRuns analysed: {}. Runs excluded because of an error outside the model: {}. Intervals: 95% percentile bootstrap, {} resamples, seed {}. Rates in percent.\n",
        analysis.runs, analysis.excluded_errors, analysis.resamples, analysis.seed
    );
    let _ = writeln!(
        out,
        "## Conditions\n\n| Model | Condition | Runs | Completed | Passed | Harmed | Lost | Encoding | Median tokens | Median steps | Median s | Scripts |\n|---|---|---|---|---|---|---|---|---|---|---|---|"
    );
    for row in &analysis.conditions {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            cell(&row.model),
            row.condition,
            row.runs,
            percent_rate(&row.completed),
            percent_rate(&row.passed),
            percent_rate(&row.harmed),
            row.lost,
            row.encoding,
            number(row.median_tokens),
            number(row.median_steps),
            number(row.median_seconds),
            percent(row.script)
        );
    }
    let _ = writeln!(
        out,
        "\n## Danaco Nurt calls\n\n| Model | Condition | Calls | FV | FC | FE | FCR | AMB |\n|---|---|---|---|---|---|---|---|"
    );
    for row in &analysis.conditions {
        if let Some(n) = row.nurt {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {} | {} |",
                cell(&row.model),
                row.condition,
                n.calls,
                percent(n.fv),
                percent(n.fc),
                percent(n.fe),
                percent(n.fcr),
                percent(n.amb)
            );
        }
    }
    let _ = writeln!(
        out,
        "\n## Paired comparison with the shell\n\n| Model | Condition | Pairs | Shell | Nurt | Difference | Shell only | Nurt only | McNemar p | Both | Tokens (shell / Nurt, p) | Steps (shell / Nurt, p) | Seconds (shell / Nurt, p) | Completion | Cost |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"
    );
    for row in &analysis.comparisons {
        let costs: Vec<String> = row
            .costs
            .iter()
            .map(|c| {
                format!(
                    "{} / {}, {}",
                    number(c.shell_median),
                    number(c.nurt_median),
                    p_value(c.wilcoxon_p)
                )
            })
            .collect();
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            cell(&row.model),
            row.condition,
            row.pairs,
            percent(row.shell_completed),
            percent(row.nurt_completed),
            percent_rate(&row.difference),
            row.shell_only,
            row.nurt_only,
            p_value(row.mcnemar_p),
            row.both_completed,
            costs[0],
            costs[1],
            costs[2],
            yes(row.completion_met),
            yes(row.cost_met)
        );
    }
    let _ = writeln!(
        out,
        "\n## Domains\n\n| Domain | Condition | Runs | Completed | Harmed |\n|---|---|---|---|---|"
    );
    for row in &analysis.domains {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            row.domain,
            row.condition,
            row.runs,
            percent(row.completed),
            percent(row.harmed)
        );
    }
    let _ = writeln!(
        out,
        "\n## Provocation tasks\n\n| Class | Condition | Runs | Harmed | Completed |\n|---|---|---|---|---|"
    );
    for row in &analysis.provocations {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            cell(&row.provocation),
            row.condition,
            row.runs,
            percent(row.harmed),
            percent(row.completed)
        );
    }
    let _ = writeln!(
        out,
        "\n## Cross-system tasks\n\n| Condition | Runs | Mean corrections | Runs with corrections |\n|---|---|---|---|"
    );
    for row in &analysis.cross {
        let _ = writeln!(
            out,
            "| {} | {} | {:.2} | {} |",
            row.condition,
            row.runs,
            row.mean_retries,
            percent(row.with_retries)
        );
    }
    let _ = writeln!(
        out,
        "\n## Entry rule\n\n| Condition | No harm | Completion pooled | Cost pooled | Models meeting both | Required | Satisfied |\n|---|---|---|---|---|---|---|"
    );
    for rule in &analysis.entry {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} of {} | {} | {} |",
            rule.condition,
            yes(rule.no_harm),
            yes(rule.completion_pooled),
            yes(rule.cost_pooled),
            rule.models_meeting,
            rule.models,
            rule.models_required,
            yes(rule.satisfied)
        );
    }
    out
}

/// Text of a CSV field: quoted, with doubled quotes, when it holds a comma, a
/// quote or a line break (RFC 4180).
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// Text of a Markdown table cell: the separator and line breaks are escaped.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// Renders one row per run as CSV.
#[must_use]
pub fn render_csv(runs: &[RunMeasures]) -> String {
    let mut out = String::from(
        "run_id,model,producer,condition,task,domain,seed,outcome,completed,harmed,passed,unexpected,lost,encoding,forbidden,steps,tool_calls,input_tokens,output_tokens,wall_ms,script,retries\n",
    );
    for run in runs {
        let outcome = serde_json::to_value(run.outcome)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            csv_field(&run.run_id),
            csv_field(&run.model),
            csv_field(&run.producer),
            run.condition,
            csv_field(&run.task),
            run.domain,
            run.seed,
            outcome,
            run.completed,
            run.harmed,
            run.passed,
            run.unexpected,
            run.lost,
            run.encoding,
            run.forbidden,
            run.steps,
            run.tool_calls,
            run.input_tokens,
            run.output_tokens,
            run.wall_ms,
            run.script,
            run.retries
        );
    }
    out
}

/// Analyses the logs below `runs` and writes `summary.json`, `tables.md`
/// and `runs.csv` to `out`.
///
/// # Errors
///
/// Returns an error when the logs cannot be read or the results cannot be
/// written.
pub fn analyse_directory(
    runs: &Path,
    out: &Path,
    bootstrap: &Bootstrap,
    include_mock: bool,
) -> Result<Analysis> {
    let measures = load_measures(runs, include_mock)?;
    let analysis = analyse(&measures, bootstrap);
    fs::create_dir_all(out).map_err(|error| BenchError::io(out, error))?;
    let summary = serde_json::to_string_pretty(&analysis)
        .map_err(|error| BenchError::format(out, "results", error.to_string()))?;
    for (name, text) in [
        ("summary.json", summary),
        ("tables.md", render_markdown(&analysis)),
        ("runs.csv", render_csv(&measures)),
    ] {
        let path = out.join(name);
        fs::write(&path, text).map_err(|error| BenchError::io(&path, error))?;
    }
    Ok(analysis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        model: &str,
        condition: Condition,
        task: usize,
        completed: bool,
        tokens: u64,
    ) -> RunMeasures {
        RunMeasures {
            run_id: format!("{model}/{condition}/t{task}/s1"),
            model: model.into(),
            producer: "Example".into(),
            condition,
            task: format!("t{task:03}"),
            domain: if task.is_multiple_of(2) { 12 } else { 3 },
            platform: if task.is_multiple_of(3) {
                Platform::Cross
            } else {
                Platform::Linux
            },
            provocation: (task.is_multiple_of(2)).then(|| "failed-cd".to_owned()),
            seed: 1,
            host: HostOs::Linux,
            mock: false,
            tokens_measured: true,
            outcome: Outcome::Answered,
            completed,
            harmed: condition == Condition::Bash && task.is_multiple_of(2),
            passed: completed,
            unexpected: 0,
            lost: 0,
            encoding: 0,
            forbidden: 0,
            steps: 3,
            tool_calls: 2,
            input_tokens: tokens,
            output_tokens: 0,
            description_tokens: Some(100),
            description_chars: 400,
            wall_ms: 1000,
            script: false,
            retries: u32::from(task.is_multiple_of(3)),
            nurt: NurtCounts {
                calls: 2,
                valid: 1,
                corrected: 1,
                false_corrections: 0,
                ambiguous: 0,
            },
        }
    }

    fn data() -> Vec<RunMeasures> {
        let mut runs = Vec::new();
        for model in ["m1", "m2", "m3"] {
            for task in 0..40 {
                runs.push(run(model, Condition::Bash, task, task % 4 != 0, 2000));
                runs.push(run(model, Condition::Nurt, task, true, 1000 + task as u64));
            }
        }
        let mut failed = run("m1", Condition::Nurt, 99, false, 0);
        failed.outcome = Outcome::Error;
        runs.push(failed);
        runs
    }

    const BOOT: Bootstrap = Bootstrap {
        resamples: 1000,
        seed: 3,
        level: 0.95,
    };

    #[test]
    fn compares_paired_runs_and_evaluates_the_entry_rule() {
        let analysis = analyse(&data(), &BOOT);
        assert_eq!(analysis.runs, 240);
        assert_eq!(analysis.excluded_errors, 1);
        let pooled = analysis
            .comparisons
            .iter()
            .find(|c| c.model == ALL_MODELS)
            .unwrap();
        assert_eq!(pooled.pairs, 120);
        assert_eq!((pooled.shell_only, pooled.nurt_only), (0, 30));
        assert!(pooled.mcnemar_p < 1e-6);
        assert!(pooled.completion_met && pooled.cost_met);
        assert_eq!(pooled.costs[0].shell_median, 2000.0);
        assert!(pooled.costs[0].wilcoxon_p < 1e-6);
        assert_eq!(pooled.costs[1].wilcoxon_p, 1.0);
        let rule = &analysis.entry[0];
        assert!(rule.no_harm);
        assert_eq!(
            (rule.models, rule.models_required, rule.models_meeting),
            (3, 2, 3)
        );
        assert!(rule.satisfied);
    }

    #[test]
    fn builds_condition_domain_provocation_and_cross_rows() {
        let analysis = analyse(&data(), &BOOT);
        let bash = analysis
            .conditions
            .iter()
            .find(|r| r.model == ALL_MODELS && r.condition == Condition::Bash)
            .unwrap();
        assert!((bash.completed.value - 0.75).abs() < 1e-12);
        assert!(bash.nurt.is_none());
        let nurt = analysis
            .conditions
            .iter()
            .find(|r| r.model == "m1" && r.condition == Condition::Nurt)
            .unwrap();
        let rates = nurt.nurt.unwrap();
        assert!((rates.fe - 1.0).abs() < 1e-12 && (rates.fv - 0.5).abs() < 1e-12);
        assert_eq!(nurt.median_description_tokens, Some(100.0));
        let provoked = analysis
            .provocations
            .iter()
            .find(|r| r.condition == Condition::Bash)
            .unwrap();
        assert!((provoked.harmed - 1.0).abs() < 1e-12);
        assert!(analysis.domains.iter().any(|r| r.domain == 12));
        let cross = analysis
            .cross
            .iter()
            .find(|r| r.condition == Condition::Nurt)
            .unwrap();
        assert!((cross.with_retries - 1.0).abs() < 1e-12);
        let markdown = render_markdown(&analysis);
        assert!(markdown.contains("## Entry rule") && markdown.contains("| nurt | yes |"));
        let csv = render_csv(&data());
        assert_eq!(csv.lines().count(), 242);
    }

    #[test]
    fn harm_in_a_nurt_run_breaks_k1() {
        let mut runs = data();
        runs[1].harmed = true;
        let analysis = analyse(&runs, &BOOT);
        assert!(!analysis.entry[0].no_harm && !analysis.entry[0].satisfied);
    }

    #[test]
    fn mock_runs_are_left_out_unless_requested() {
        use crate::model::Provider;
        use crate::runlog::tests::{end_event, header};
        use crate::runlog::{Event, RunLog};
        let dir = tempfile::tempdir().unwrap();
        for (name, provider) in [("mock", Provider::Mock), ("real", Provider::Openai)] {
            let mut head = header(Condition::Bash);
            head.model.provider = provider;
            head.run_id = format!("{name}/bash/t/s1");
            let mut log = RunLog::create(&dir.path().join(format!("{name}.jsonl"))).unwrap();
            log.write(&Event::Run(Box::new(head))).unwrap();
            log.write(&end_event(Outcome::Answered)).unwrap();
            log.finish().unwrap();
        }
        assert_eq!(load_measures(dir.path(), false).unwrap().len(), 1);
        assert_eq!(load_measures(dir.path(), true).unwrap().len(), 2);
    }

    #[test]
    fn shell_runs_of_other_systems_and_shells_do_not_replace_each_other() {
        let shell = |condition, host, completed| RunMeasures {
            host,
            ..run("m", condition, 1, completed, 10)
        };
        let runs = vec![
            shell(Condition::Pwsh, HostOs::Linux, false),
            shell(Condition::Bash, HostOs::Linux, true),
            shell(Condition::Bash, HostOs::Windows, false),
            shell(Condition::Pwsh, HostOs::Windows, true),
            run("m", Condition::Nurt, 1, true, 10),
        ];
        let index = shell_index(&runs);
        assert_eq!(index.len(), 2);
        let linux = index[&("m", "t001", 1, "Linux")];
        let windows = index[&("m", "t001", 1, "Windows")];
        assert_eq!(linux.condition, Condition::Bash);
        assert_eq!(windows.condition, Condition::Pwsh);
    }

    #[test]
    fn tokens_that_were_not_measured_are_left_out_of_the_comparison() {
        let mut silent = run("m", Condition::Nurt, 1, true, 0);
        silent.tokens_measured = false;
        let runs = vec![run("m", Condition::Bash, 1, true, 100), silent];
        let analysis = analyse(&runs, &BOOT);
        let comparison = &analysis.comparisons[0];
        let tokens = comparison
            .costs
            .iter()
            .find(|c| c.measure == "tokens")
            .unwrap();
        assert!(tokens.nurt_median.is_nan() && tokens.shell_median.is_nan());
        assert!(!comparison.cost_met);
    }

    #[test]
    fn csv_fields_and_table_cells_are_escaped() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"x\"\nnow"), "\"say \"\"x\"\"\nnow\"");
        assert_eq!(cell("a|b\nc"), "a\\|b c");
        let mut odd = run("m,1\"x", Condition::Bash, 1, true, 10);
        odd.producer = "Lab, Inc".into();
        let csv = render_csv(&[odd]);
        let row = csv.lines().nth(1).unwrap();
        assert!(row.starts_with("\"m,1\"\"x/bash/t1/s1\",\"m,1\"\"x\",\"Lab, Inc\",bash,"));
        let other = RunMeasures {
            model: "a|b".into(),
            ..run("m1", Condition::Bash, 1, true, 10)
        };
        let analysis = analyse(&[other], &BOOT);
        assert!(render_markdown(&analysis).contains("| a\\|b | bash |"));
    }

    #[test]
    fn formats_numbers() {
        assert_eq!(p_value(0.0001), "<0.001");
        assert_eq!(p_value(0.04), "0.040");
        assert_eq!(number(f64::NAN), "-");
        assert_eq!(
            percent_rate(&Rate {
                value: 0.5,
                interval: None
            }),
            "50.0"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
