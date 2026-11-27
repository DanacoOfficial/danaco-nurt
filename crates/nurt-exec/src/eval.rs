// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding of bound arguments to the expression evaluator.

use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use nurt_contracts::bound::{BoundExpr, BoundStage};
use nurt_contracts::env::Clock;
use nurt_contracts::error::Result;
use nurt_contracts::stage::{ArgValue, BoundArgs, ExecCtx, ExprEvaluator};
use nurt_contracts::value::{Fields, Record, Value};
use nurt_types::{ClockEnv, Evaluator, RandomSource};

/// Functions whose value differs between calls with the same arguments; an
/// argument that calls one is evaluated per record.
const PER_RECORD_FUNCTIONS: [&str; 2] = ["random", "uuid"];

/// Evaluator of the expressions of bound arguments over the evaluator of the
/// language, with the named results of the execution.
///
/// The executor installs one binding per execution as the
/// [`ExprEvaluator`] of every stage context, so that
/// [`BoundArgs::eval`] computes `ƒ` parameters per record. Named results
/// are snapshots: [`ExprBinding::define`] stores the value once and every
/// later reference reads the same value.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::ast::BinOp;
/// use nurt_contracts::bound::BoundExpr;
/// use nurt_contracts::value::{Record, RecordKind, Value};
/// use nurt_exec::ExprBinding;
/// use nurt_testkit::clock::VirtualClock;
///
/// let binding = ExprBinding::with_seed(7);
/// binding.define("limit", Value::Int(10));
/// let expr = BoundExpr::Bin {
///     op: BinOp::Lt,
///     l: Box::new(BoundExpr::Field { name: "size".to_owned(), of: None }),
///     r: Box::new(BoundExpr::Ref(nurt_contracts::bound::BoundRef {
///         name: "limit".to_owned(),
///         fields: Vec::new(),
///     })),
/// };
/// let record = Record::new(RecordKind::Entry).with("size", Value::Int(3));
/// let clock = VirtualClock::new();
/// assert_eq!(binding.evaluate(&expr, Some(&record), &clock)?, Value::Bool(true));
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub struct ExprBinding {
    evaluator: Evaluator,
    random: RandomSource,
    names: RwLock<Arc<Fields>>,
}

impl ExprBinding {
    /// Binding with random numbers seeded from the entropy of the process.
    #[must_use]
    pub fn new() -> Self {
        Self::with_random(RandomSource::new())
    }

    /// Binding with a fixed seed of `random()` and `uuid()`.
    #[must_use]
    pub fn with_seed(seed: u64) -> Self {
        Self::with_random(RandomSource::with_seed(seed))
    }

    fn with_random(random: RandomSource) -> Self {
        Self {
            evaluator: Evaluator::new(),
            random,
            names: RwLock::new(Arc::new(Fields::new())),
        }
    }

    /// Named results defined so far, by name without `@`.
    #[must_use]
    pub fn names(&self) -> Arc<Fields> {
        Arc::clone(&self.names.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Value of the named result `@name`.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<Value> {
        self.names().get(name).cloned()
    }

    /// Stores the snapshot of a named result; a later definition of the same
    /// name replaces it.
    /// Replaces every name, returning the names that were defined: a
    /// function sees its parameters and nothing of its caller.
    pub fn replace_names(&self, names: Fields) -> Arc<Fields> {
        let mut current = self.names.write().unwrap_or_else(PoisonError::into_inner);
        std::mem::replace(&mut *current, Arc::new(names))
    }

    /// Restores the names returned by [`ExprBinding::names`] or
    /// [`ExprBinding::replace_names`]: the names defined since are dropped.
    pub fn restore_names(&self, names: Arc<Fields>) {
        *self.names.write().unwrap_or_else(PoisonError::into_inner) = names;
    }

    /// Defines a name for the value, replacing a name of the same spelling.
    pub fn define(&self, name: impl Into<String>, value: Value) {
        let mut names = self.names.write().unwrap_or_else(PoisonError::into_inner);
        Arc::make_mut(&mut names).insert(name.into(), value);
    }

    /// Value of `expr` for `record`, with the time read from `clock`.
    ///
    /// # Errors
    ///
    /// Fails with the errors of the evaluator of the language, for example
    /// `E_UNKNOWN_FIELD`, `E_UNKNOWN_NAME` or `E_TYPE`.
    pub fn evaluate(
        &self,
        expr: &BoundExpr,
        record: Option<&Record>,
        clock: &dyn Clock,
    ) -> Result<Value> {
        let names = self.names();
        let env = ClockEnv::new(clock, &self.random).with_names(&names);
        self.evaluator.eval(expr, record, &env)
    }

    /// Bound arguments of a stage.
    ///
    /// Every argument of the bound stage is present in signature order with
    /// its default mark. A `null` value is absent, a sub-pipeline is a
    /// pipeline argument, an expression that reads the current record or
    /// calls a function with a new value per call stays an expression
    /// evaluated per record, and every other expression, including references
    /// to named results, is evaluated once here.
    ///
    /// # Errors
    ///
    /// Fails with the error of the evaluation of a constant argument, for
    /// example `E_UNKNOWN_NAME` for an unknown named result.
    pub fn bind(&self, stage: &BoundStage, clock: &dyn Clock) -> Result<BoundArgs> {
        let mut args = BoundArgs::new(stage.name.clone());
        for arg in &stage.args {
            let value = match &arg.value {
                BoundExpr::Null => ArgValue::Absent,
                BoundExpr::Sub { p } => ArgValue::Pipeline(p.clone()),
                expr if is_per_record(expr) => ArgValue::Expr(expr.clone()),
                expr => ArgValue::Value(self.evaluate(expr, None, clock)?),
            };
            args.insert(arg.name.clone(), value, arg.default);
        }
        Ok(args)
    }
}

impl Default for ExprBinding {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ExprBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.names();
        f.debug_struct("ExprBinding")
            .field("names", &names.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl ExprEvaluator for ExprBinding {
    fn eval(&self, expr: &BoundExpr, record: Option<&Record>, ctx: &ExecCtx) -> Result<Value> {
        self.evaluate(expr, record, ctx.clock())
    }
}

/// Returns `true` when the value of `expr` depends on the current record or
/// changes between evaluations: it reads a field of the record or calls
/// `random()` or `uuid()`.
///
/// ```
/// use nurt_contracts::bound::BoundExpr;
/// use nurt_exec::is_per_record;
///
/// assert!(is_per_record(&BoundExpr::Field { name: "size".to_owned(), of: None }));
/// assert!(!is_per_record(&BoundExpr::Int { v: 3 }));
/// ```
#[must_use]
pub fn is_per_record(expr: &BoundExpr) -> bool {
    match expr {
        BoundExpr::Field { of: None, .. } => true,
        BoundExpr::Field { of: Some(base), .. } => is_per_record(base),
        BoundExpr::Call { function, args } => {
            PER_RECORD_FUNCTIONS.contains(&function.as_str()) || args.iter().any(is_per_record)
        }
        BoundExpr::Un { x, .. } | BoundExpr::Spread { x } => is_per_record(x),
        BoundExpr::Bin { l, r, .. } => is_per_record(l) || is_per_record(r),
        BoundExpr::List { items } => items.iter().any(is_per_record),
        BoundExpr::Map { entries } => entries.iter().any(|entry| is_per_record(&entry.v)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::ast::BinOp;
    use nurt_contracts::bound::{
        BoundArg, BoundHead, BoundMapEntry, BoundPipeline, BoundRef, Phase,
    };
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::kinds::EffectKind;
    use nurt_contracts::value::RecordKind;
    use nurt_testkit::clock::VirtualClock;

    use super::*;

    fn field(name: &str) -> BoundExpr {
        BoundExpr::Field {
            name: name.to_owned(),
            of: None,
        }
    }

    fn reference(name: &str) -> BoundExpr {
        BoundExpr::Ref(BoundRef {
            name: name.to_owned(),
            fields: Vec::new(),
        })
    }

    fn stage(args: Vec<BoundArg>) -> BoundStage {
        BoundStage {
            name: "filter".to_owned(),
            effect: EffectKind::Read,
            phase: Phase::Transform,
            waits: false,
            args,
            span: None,
        }
    }

    #[test]
    fn classifies_record_dependent_expressions() {
        let call = |function: &str, args: Vec<BoundExpr>| BoundExpr::Call {
            function: function.to_owned(),
            args,
        };
        assert!(is_per_record(&call("uuid", Vec::new())));
        assert!(is_per_record(&call("lower", vec![field("name")])));
        assert!(!is_per_record(&call(
            "lower",
            vec![BoundExpr::Text { v: "A".to_owned() }]
        )));
        assert!(is_per_record(&BoundExpr::List {
            items: vec![BoundExpr::Int { v: 1 }, field("size")],
        }));
        assert!(is_per_record(&BoundExpr::Map {
            entries: vec![BoundMapEntry {
                k: "a".to_owned(),
                v: field("size"),
            }],
        }));
        assert!(is_per_record(&BoundExpr::Field {
            name: "x".to_owned(),
            of: Some(Box::new(field("meta"))),
        }));
        assert!(!is_per_record(&BoundExpr::Field {
            name: "x".to_owned(),
            of: Some(Box::new(reference("meta"))),
        }));
        assert!(!is_per_record(&reference("files")));
    }

    #[test]
    fn binds_constants_expressions_pipelines_and_absent_values() {
        let binding = ExprBinding::with_seed(1);
        binding.define("limit", Value::Int(4));
        let clock = VirtualClock::new();
        let pipeline = BoundPipeline {
            head: BoundHead::Ref(BoundRef {
                name: "limit".to_owned(),
                fields: Vec::new(),
            }),
            stages: Vec::new(),
            span: None,
        };
        let condition = BoundExpr::Bin {
            op: BinOp::Gt,
            l: Box::new(field("size")),
            r: Box::new(reference("limit")),
        };
        let args = binding
            .bind(
                &stage(vec![
                    BoundArg::given("where", condition.clone()),
                    BoundArg::given("max", reference("limit")),
                    BoundArg::from_default("label", BoundExpr::Null),
                    BoundArg::given("check", BoundExpr::Sub { p: pipeline }),
                ]),
                &clock,
            )
            .unwrap();
        assert_eq!(args.expr("where"), Some(&condition));
        assert_eq!(args.value("max"), Some(&Value::Int(4)));
        assert_eq!(args.get("label"), Some(&ArgValue::Absent));
        assert!(args.is_defaulted("label"));
        assert!(args.pipeline("check").is_some());
    }

    #[test]
    fn unknown_named_result_fails_binding() {
        let binding = ExprBinding::new();
        let clock = VirtualClock::new();
        let error = binding
            .bind(
                &stage(vec![BoundArg::given("max", reference("none"))]),
                &clock,
            )
            .err()
            .map(|error| error.code());
        assert_eq!(error, Some(ErrorCode::UnknownName));
    }

    #[test]
    fn later_definition_replaces_snapshot_without_changing_earlier_reads() {
        let binding = ExprBinding::default();
        binding.define("n", Value::Int(1));
        let before = binding.names();
        binding.define("n", Value::Int(2));
        assert_eq!(before.get("n"), Some(&Value::Int(1)));
        assert_eq!(binding.named("n"), Some(Value::Int(2)));
        assert!(format!("{binding:?}").contains('n'));
    }

    #[test]
    fn evaluates_per_record_through_the_contract_interface() {
        let binding = Arc::new(ExprBinding::new());
        let clock = Arc::new(VirtualClock::new());
        let ctx = ExecCtx::builder(clock, Arc::new(nurt_testkit::host::TestHost::linux()))
            .evaluator(binding)
            .build();
        let args = BoundArgs::new("filter").with("size", ArgValue::Expr(field("size")), false);
        let record = Record::new(RecordKind::Entry).with("size", Value::Int(9));
        let value = args.eval("size", Some(&record), &ctx).unwrap();
        assert_eq!(value, Some(Value::Int(9)));
    }
}
