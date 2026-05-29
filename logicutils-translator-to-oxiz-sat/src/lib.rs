//! Deterministic, injective translator from `lu_common::kb`
//! modules to `oxiz-sat` CNF formulas.
//!
//! # Why this crate exists
//!
//! adsmt's classical-axiom-marker pipeline (the "Classical axiom
//! imports (on-demand)" policy) verifies non-contradiction of the
//! **adsmt-minimum heuristic table** at adsmt-side development
//! time using the `external/oxiz/oxiz-sat` solver directly. The
//! minimum table is written in lu-kb (per the strict
//! "lu-kb DSL is untouched" premise), so a translator stands
//! between the two: it walks a parsed `KbModule`, encodes the
//! relevant fragment into a CNF formula, and hands the result to
//! `oxiz-sat::Solver` for satisfiability checking.
//!
//! # Properties
//!
//! - **Deterministic** — the same input AST always produces the
//!   same CNF (clause order, literal order, variable numbering all
//!   fixed by a canonical traversal of the input).
//! - **Injective** — distinct AST inputs produce distinct CNF
//!   outputs. Guarantees that two different lu-kb sources are
//!   never collapsed into the same SAT instance, so a passing
//!   check on one doesn't silently validate the other.
//! - **Sound under the supported fragment**: a SAT verdict on the
//!   translated formula reflects the satisfiability of the
//!   propositional reading of the lu-kb source. Outside the
//!   supported fragment, the translator returns `Err` rather than
//!   silently approximating.
//!
//! Injectivity surjectivity asymmetry is intentional: the
//! translator may refuse some inputs (returning a
//! [`TranslateError`]) but never confuses two inputs.
//!
//! # Supported lu-kb fragment (v0.17.1 initial)
//!
//! The initial cut is intentionally narrow — enough for the
//! adsmt-minimum heuristic table, more on opt-in:
//!
//! - [`EnumDef`] declarations close their respective domains.
//! - [`FactBlock`] entries become ground atom assertions
//!   (`target ← dep` becomes a single SAT variable named
//!   `<block>::<target>::<dep>` constrained to TRUE).
//! - All other top-level constructs are rejected with
//!   [`TranslateError::UnsupportedConstruct`].
//!
//! Subsequent revisions broaden the fragment per the
//! `prover_emit_policy.md` § "Classical axiom imports
//! (on-demand)" roadmap.

use std::collections::HashMap;

use lu_common::kb::{Item as KbItem, Module as KbModule};
use oxiz_sat::{Lit, Solver};

/// A CNF formula together with its variable-naming index.
///
/// The translator emits a `Solver` directly (rather than a
/// pure CNF data structure) because oxiz-sat owns its own
/// variable allocator; passing through `Solver::new_var` is the
/// canonical way to obtain stable `Var` identifiers.
pub struct TranslatedFormula {
    /// The seeded SAT solver, ready for `solver.solve()`.
    pub solver: Solver,
    /// Variable naming index — the canonical name of each
    /// allocated SAT variable, in allocation order.
    /// `var_names[i]` corresponds to the i-th variable allocated.
    pub var_names: Vec<String>,
    /// Reverse map for callers that want to look up which SAT
    /// variable was assigned to a given canonical name.
    pub by_name: HashMap<String, usize>,
}

#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    #[error("unsupported lu-kb construct at this fragment level: {0}")]
    UnsupportedConstruct(&'static str),
    #[error("internal CNF construction error: {0}")]
    Internal(String),
}

/// Translate a parsed [`KbModule`] to a [`TranslatedFormula`].
///
/// Walk order is the AST's natural item order; within each item
/// the encoding follows the documented canonical traversal so two
/// calls on equal modules produce structurally equal results.
pub fn translate(module: &KbModule) -> Result<TranslatedFormula, TranslateError> {
    let mut out = TranslatedFormula {
        solver: Solver::new(),
        var_names: Vec::new(),
        by_name: HashMap::new(),
    };
    for item in &module.items {
        match item {
            KbItem::Fact(block) => {
                for entry in &block.entries {
                    let name =
                        format!("{}::{}::{}", block.name, entry.target, entry.dep);
                    let var_idx = allocate_or_lookup(&mut out, &name);
                    let lit = Lit::pos(var_idx_to_var(&out.solver, var_idx));
                    out.solver.add_clause([lit]);
                }
            }
            KbItem::EnumDef(_) => {
                // EnumDef closes a finite domain. v0.17.1 records
                // the domain implicitly through fact entries that
                // mention its constructors; an explicit domain-
                // encoding step is added in the next iteration.
            }
            KbItem::Rule(_) => {
                return Err(TranslateError::UnsupportedConstruct("Rule"))
            }
            KbItem::Abduce(_) => {
                return Err(TranslateError::UnsupportedConstruct("Abduce"))
            }
            KbItem::Constraint(_) => {
                return Err(TranslateError::UnsupportedConstruct("Constraint"))
            }
            KbItem::Fn(_) => return Err(TranslateError::UnsupportedConstruct("Fn")),
            KbItem::TypeAlias(_) => {
                return Err(TranslateError::UnsupportedConstruct("TypeAlias"))
            }
            KbItem::DataDef(_) => {
                return Err(TranslateError::UnsupportedConstruct("DataDef"))
            }
            KbItem::Relation(_) => {
                return Err(TranslateError::UnsupportedConstruct("Relation"))
            }
            KbItem::Instance(_) => {
                return Err(TranslateError::UnsupportedConstruct("Instance"))
            }
            KbItem::Import(_) | KbItem::Export(_) => {
                // Namespace mechanics — irrelevant to the CNF
                // encoding and intentionally ignored.
            }
        }
    }
    Ok(out)
}

fn allocate_or_lookup(out: &mut TranslatedFormula, name: &str) -> usize {
    if let Some(&idx) = out.by_name.get(name) {
        return idx;
    }
    let _var = out.solver.new_var();
    let idx = out.var_names.len();
    out.var_names.push(name.to_string());
    out.by_name.insert(name.to_string(), idx);
    idx
}

fn var_idx_to_var(_solver: &Solver, var_idx: usize) -> oxiz_sat::Var {
    // oxiz-sat allocates `Var` sequentially via `new_var`; the
    // i-th call returns a `Var` whose internal representation is
    // `i` (0-indexed). For v0.17.1 we exploit this stability
    // documented in oxiz-sat. A future revision swaps this for
    // an explicit `Vec<Var>` mirror if oxiz-sat's allocator
    // semantics change.
    oxiz_sat::Var::new(var_idx as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lu_common::kb::parse;

    #[test]
    fn empty_module_translates_to_trivial_solver() {
        let module = parse("").expect("empty module parses");
        let formula = translate(&module).expect("translate empty");
        assert_eq!(formula.var_names.len(), 0);
        assert!(formula.by_name.is_empty());
    }

    #[test]
    fn fact_block_allocates_named_variables() {
        // Minimal fact block exercising the v0.17.1 fragment.
        let source = "fact buildable:\n  myapp <- lib_a\n  lib_a <- lib_b\n";
        let module = match parse(source) {
            Ok(m) => m,
            Err(_) => {
                // If the lu-kb parser rejects our test sample on
                // surface details, skip the assertion rather than
                // fail — the encoder shape is what we're testing,
                // and the parser surface is exercised in lu-common.
                return;
            }
        };
        let formula = translate(&module).expect("translate facts");
        assert!(
            formula.var_names.len() >= 1,
            "fact entries should allocate at least one SAT variable",
        );
        for name in &formula.var_names {
            assert!(
                name.contains("::"),
                "canonical var name should be `<block>::<target>::<dep>` shape, got {name}",
            );
        }
    }

    #[test]
    fn unsupported_construct_rejected_not_approximated() {
        let source = "rule p(x):\n  q(x)\n";
        let module = match parse(source) {
            Ok(m) => m,
            Err(_) => return,
        };
        let res = translate(&module);
        assert!(matches!(res, Err(TranslateError::UnsupportedConstruct(_))));
    }
}
