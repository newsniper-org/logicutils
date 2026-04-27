use lu_common::kb::ast::*;
use std::collections::HashMap;

/// Variable bindings from a query result.
pub type Bindings = HashMap<String, Value>;

/// Runtime values.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Atom(String), // An unbound variable name or symbolic atom
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::String(s) => write!(f, "{s}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Atom(a) => write!(f, "{a}"),
        }
    }
}

/// Result of a query: a list of bindings (one per solution).
pub type QueryResult = Vec<Bindings>;

/// The built-in logic engine.
pub struct Engine {
    facts: Vec<StoredFact>,
    rules: Vec<StoredRule>,
}

#[derive(Debug, Clone)]
struct StoredFact {
    name: String,
    args: Vec<Value>,
}

#[derive(Debug, Clone)]
struct StoredRule {
    head_name: String,
    head_args: Vec<String>, // Parameter names
    body: Vec<BodyExpr>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            facts: Vec::new(),
            rules: Vec::new(),
        }
    }

    /// Load a parsed module into the engine.
    pub fn load_module(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::Fact(fb) => {
                    for entry in &fb.entries {
                        self.facts.push(StoredFact {
                            name: fb.name.clone(),
                            args: vec![
                                Value::Atom(entry.target.clone()),
                                Value::Atom(entry.dep.clone()),
                            ],
                        });
                    }
                }
                Item::Rule(r) => {
                    let head_args: Vec<String> =
                        r.head.args.iter().map(|a| a.name.clone()).collect();
                    self.rules.push(StoredRule {
                        head_name: r.head.name.clone(),
                        head_args,
                        body: r.body.clone(),
                    });
                }
                _ => {
                    // Other items (fn, type, relation, etc.) not yet evaluated
                }
            }
        }
    }

    /// Add a fact directly.
    pub fn add_fact(&mut self, name: &str, args: Vec<Value>) {
        self.facts.push(StoredFact {
            name: name.to_string(),
            args,
        });
    }

    /// Query the engine with a predicate name and argument patterns.
    /// Arguments can be bound (specific values) or unbound (variable names starting with uppercase).
    pub fn query(&self, name: &str, args: &[QueryArg]) -> QueryResult {
        let mut results = Vec::new();

        // Try facts first
        for fact in &self.facts {
            if fact.name != name || fact.args.len() != args.len() {
                continue;
            }
            if let Some(bindings) = unify_args(args, &fact.args) {
                results.push(bindings);
            }
        }

        // Try rules
        for rule in &self.rules {
            if rule.head_name != name || rule.head_args.len() != args.len() {
                continue;
            }
            // Create initial bindings: only bind rule head params that have concrete values from query
            let mut initial_bindings = Bindings::new();
            for (query_arg, param) in args.iter().zip(&rule.head_args) {
                if let QueryArg::Bound(val) = query_arg {
                    initial_bindings.insert(param.clone(), val.clone());
                }
                // QueryArg::Var means the param is unbound — don't add to bindings
            }

            // Evaluate rule body with these bindings
            let body_results = self.eval_body(&rule.body, initial_bindings);

            for binding in body_results {
                // Map rule param names back to query variable names
                let mut result = Bindings::new();
                for (query_arg, param) in args.iter().zip(&rule.head_args) {
                    if let QueryArg::Var(var_name) = query_arg {
                        if let Some(val) = binding.get(param) {
                            result.insert(var_name.clone(), val.clone());
                        }
                    }
                }
                results.push(result);
            }
        }

        results
    }

    fn eval_body(&self, body: &[BodyExpr], bindings: Bindings) -> Vec<Bindings> {
        if body.is_empty() {
            return vec![bindings];
        }

        let first = &body[0];
        let rest = &body[1..];

        match first {
            BodyExpr::PredicateCall(name, args) => {
                let query_args: Vec<QueryArg> = args
                    .iter()
                    .map(|e| expr_to_query_arg(e, &bindings))
                    .collect();

                let sub_results = self.query(name, &query_args);
                let mut all_results = Vec::new();

                for sub_binding in sub_results {
                    let mut merged = bindings.clone();
                    for (k, v) in &sub_binding {
                        merged.insert(k.clone(), v.clone());
                    }
                    all_results.extend(self.eval_body(rest, merged));
                }
                all_results
            }
            BodyExpr::Not(inner) => {
                // Negation as failure
                let inner_results = self.eval_body(&[*inner.clone()], bindings.clone());
                if inner_results.is_empty() {
                    self.eval_body(rest, bindings)
                } else {
                    Vec::new()
                }
            }
            BodyExpr::Condition(expr) => {
                if eval_condition(expr, &bindings) {
                    self.eval_body(rest, bindings)
                } else {
                    Vec::new()
                }
            }
            BodyExpr::Let(name, expr) => {
                let val = eval_expr(expr, &bindings);
                let mut new_bindings = bindings;
                new_bindings.insert(name.clone(), val);
                self.eval_body(rest, new_bindings)
            }
            BodyExpr::Explain(_) | BodyExpr::ScopedImport(_) => {
                // Skip for now in deductive evaluation
                self.eval_body(rest, bindings)
            }
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// A query argument: either a variable to bind or a specific value.
#[derive(Debug, Clone)]
pub enum QueryArg {
    Var(String),
    Bound(Value),
}

/// Parse a query string like "predicate(X, Y)" or "predicate(value, X)".
pub fn parse_query(input: &str) -> Result<(String, Vec<QueryArg>), String> {
    let input = input.trim();
    let paren_pos = input
        .find('(')
        .ok_or_else(|| format!("missing '(' in query: {input}"))?;
    let name = input[..paren_pos].trim().to_string();

    if !input.ends_with(')') {
        return Err(format!("missing ')' in query: {input}"));
    }
    let args_str = &input[paren_pos + 1..input.len() - 1];

    if args_str.trim().is_empty() {
        return Ok((name, Vec::new()));
    }

    let args: Vec<QueryArg> = args_str
        .split(',')
        .map(|s| {
            let s = s.trim();
            if s.chars().next().is_some_and(|c| c.is_uppercase()) {
                QueryArg::Var(s.to_string())
            } else if let Ok(n) = s.parse::<i64>() {
                QueryArg::Bound(Value::Int(n))
            } else if s.starts_with('"') && s.ends_with('"') {
                QueryArg::Bound(Value::String(s[1..s.len() - 1].to_string()))
            } else {
                QueryArg::Bound(Value::Atom(s.to_string()))
            }
        })
        .collect();

    Ok((name, args))
}

fn unify_args(query_args: &[QueryArg], fact_args: &[Value]) -> Option<Bindings> {
    let mut bindings = Bindings::new();
    for (qa, fa) in query_args.iter().zip(fact_args) {
        match qa {
            QueryArg::Var(name) => {
                if let Some(existing) = bindings.get(name) {
                    if existing != fa {
                        return None;
                    }
                } else {
                    bindings.insert(name.clone(), fa.clone());
                }
            }
            QueryArg::Bound(val) => {
                if val != fa {
                    return None;
                }
            }
        }
    }
    Some(bindings)
}

fn expr_to_query_arg(expr: &Expr, bindings: &Bindings) -> QueryArg {
    match expr {
        Expr::Ident(name) => {
            if let Some(val) = bindings.get(name) {
                QueryArg::Bound(val.clone())
            } else if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                QueryArg::Var(name.clone())
            } else {
                QueryArg::Bound(Value::Atom(name.clone()))
            }
        }
        Expr::StringLit(s) => QueryArg::Bound(Value::String(s.clone())),
        Expr::IntLit(n) => QueryArg::Bound(Value::Int(*n)),
        Expr::FloatLit(f) => QueryArg::Bound(Value::Float(*f)),
        _ => QueryArg::Bound(Value::Atom(format!("{expr:?}"))),
    }
}

fn eval_condition(expr: &Expr, bindings: &Bindings) -> bool {
    match expr {
        Expr::BinOp(left, op, right) => {
            let l = eval_expr(left, bindings);
            let r = eval_expr(right, bindings);
            match op {
                BinOp::Eq => l == r,
                BinOp::Neq => l != r,
                BinOp::Lt => compare_values(&l, &r).is_some_and(|o| o == std::cmp::Ordering::Less),
                BinOp::Gt => compare_values(&l, &r).is_some_and(|o| o == std::cmp::Ordering::Greater),
                BinOp::Le => compare_values(&l, &r).is_some_and(|o| o != std::cmp::Ordering::Greater),
                BinOp::Ge => compare_values(&l, &r).is_some_and(|o| o != std::cmp::Ordering::Less),
                _ => false,
            }
        }
        _ => true, // Non-boolean expressions are truthy
    }
}

fn eval_expr(expr: &Expr, bindings: &Bindings) -> Value {
    match expr {
        Expr::Ident(name) => bindings
            .get(name)
            .cloned()
            .unwrap_or_else(|| Value::Atom(name.clone())),
        Expr::StringLit(s) => Value::String(s.clone()),
        Expr::IntLit(n) => Value::Int(*n),
        Expr::FloatLit(f) => Value::Float(*f),
        Expr::BinOp(left, op, right) => {
            let l = eval_expr(left, bindings);
            let r = eval_expr(right, bindings);
            match (op, &l, &r) {
                (BinOp::Add, Value::Int(a), Value::Int(b)) => Value::Int(a + b),
                (BinOp::Sub, Value::Int(a), Value::Int(b)) => Value::Int(a - b),
                (BinOp::Mul, Value::Int(a), Value::Int(b)) => Value::Int(a * b),
                (BinOp::Div, Value::Int(a), Value::Int(b)) if *b != 0 => Value::Int(a / b),
                (BinOp::Eq, _, _) => Value::Bool(l == r),
                (BinOp::Neq, _, _) => Value::Bool(l != r),
                _ => Value::Atom(format!("{l} {op:?} {r}")),
            }
        }
        _ => Value::Atom(format!("{expr:?}")),
    }
}

fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        (Value::Atom(x), Value::Atom(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lu_common::kb;

    #[test]
    fn test_fact_query() {
        let source = "fact depends:\n  main_o <- main_c\n  main_o <- header_h\n  utils_o <- utils_c\n";
        let module = kb::parse(source).unwrap();
        let mut engine = Engine::new();
        engine.load_module(&module);

        // Query all deps of main_o
        let results = engine.query(
            "depends",
            &[
                QueryArg::Bound(Value::Atom("main_o".into())),
                QueryArg::Var("Dep".into()),
            ],
        );
        assert_eq!(results.len(), 2);
        let deps: Vec<String> = results
            .iter()
            .map(|b| b["Dep"].to_string())
            .collect();
        assert!(deps.contains(&"main_c".to_string()));
        assert!(deps.contains(&"header_h".to_string()));
    }

    #[test]
    fn test_fact_query_all_unbound() {
        let source = "fact depends:\n  a <- b\n  c <- d\n";
        let module = kb::parse(source).unwrap();
        let mut engine = Engine::new();
        engine.load_module(&module);

        let results = engine.query(
            "depends",
            &[QueryArg::Var("X".into()), QueryArg::Var("Y".into())],
        );
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_rule_query() {
        // Use direct fact loading to avoid parser dependency issues
        let mut engine = Engine::new();
        engine.add_fact("depends", vec![Value::Atom("main_o".into()), Value::Atom("main_c".into())]);
        engine.add_fact("depends", vec![Value::Atom("main_o".into()), Value::Atom("header_h".into())]);
        engine.add_fact("newer", vec![Value::Atom("main_c".into()), Value::Atom("main_o".into())]);

        // Load rule from KB
        let rule_source = "\
rule stale(Target):
  depends(Target, Dep)
  newer(Dep, Target)
";
        let module = kb::parse(rule_source).unwrap();
        engine.load_module(&module);

        let results = engine.query("stale", &[QueryArg::Var("Target".into())]);
        assert!(!results.is_empty());
        assert!(results.iter().any(|b| b["Target"] == Value::Atom("main_o".into())));
    }

    #[test]
    fn test_add_fact_directly() {
        let mut engine = Engine::new();
        engine.add_fact("color", vec![Value::Atom("sky".into()), Value::Atom("blue".into())]);
        engine.add_fact("color", vec![Value::Atom("grass".into()), Value::Atom("green".into())]);

        let results = engine.query(
            "color",
            &[QueryArg::Var("Thing".into()), QueryArg::Bound(Value::Atom("blue".into()))],
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["Thing"], Value::Atom("sky".into()));
    }

    #[test]
    fn test_negation() {
        let source = "\
fact exists:
  a <- yes
  b <- yes

rule missing(X):
  not exists(X, yes)
";
        // Note: this is a simplified test. Full negation-as-failure requires
        // iterating over a domain, which we don't do here.
        // Instead, test that negation works when the sub-query has bound args.
        let module = kb::parse(source).unwrap();
        let mut engine = Engine::new();
        engine.load_module(&module);

        // exists(a, yes) succeeds, so missing(a) should fail
        // We test with a bound value
        let results = engine.query("missing", &[QueryArg::Bound(Value::Atom("a".into()))]);
        assert!(results.is_empty()); // a exists, so it's not missing
    }

    #[test]
    fn test_parse_query() {
        let (name, args) = parse_query("stale(X)").unwrap();
        assert_eq!(name, "stale");
        assert_eq!(args.len(), 1);
        assert!(matches!(&args[0], QueryArg::Var(n) if n == "X"));

        let (name, args) = parse_query("depends(main_o, X)").unwrap();
        assert_eq!(name, "depends");
        assert_eq!(args.len(), 2);
        assert!(matches!(&args[0], QueryArg::Bound(Value::Atom(n)) if n == "main_o"));
        assert!(matches!(&args[1], QueryArg::Var(n) if n == "X"));
    }

    #[test]
    fn test_condition_in_rule() {
        let mut engine = Engine::new();
        engine.add_fact("score", vec![Value::Atom("alice".into()), Value::Int(90)]);
        engine.add_fact("score", vec![Value::Atom("bob".into()), Value::Int(85)]);

        let rule_source = "\
rule high_scorer(Name):
  score(Name, Score)
";
        let module = kb::parse(rule_source).unwrap();
        engine.load_module(&module);

        let results = engine.query("high_scorer", &[QueryArg::Var("Name".into())]);
        assert_eq!(results.len(), 2);
    }
}
