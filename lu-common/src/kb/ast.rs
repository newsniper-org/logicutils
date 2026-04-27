/// A complete KB module.
#[derive(Debug, Clone)]
pub struct Module {
    pub items: Vec<Item>,
}

/// Top-level items in a KB file.
#[derive(Debug, Clone)]
pub enum Item {
    Import(Import),
    Export(Export),
    Fact(FactBlock),
    Rule(RuleDecl),
    Abduce(AbduceDecl),
    Constraint(ConstraintDecl),
    Fn(FnDecl),
    TypeAlias(TypeAlias),
    DataDef(DataDef),
    Relation(RelationDecl),
    Instance(InstanceDecl),
}

// === Imports / Exports ===

#[derive(Debug, Clone)]
pub struct Import {
    pub path: Vec<String>,
    pub names: Option<Vec<String>>, // None = import all, Some = selective
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub path: Vec<String>,
}

// === Facts ===

#[derive(Debug, Clone)]
pub struct FactBlock {
    pub name: String,
    pub entries: Vec<FactEntry>,
}

#[derive(Debug, Clone)]
pub struct FactEntry {
    pub target: String,
    pub dep: String,
}

// === Rules (Deductive) ===

#[derive(Debug, Clone)]
pub struct RuleDecl {
    pub head: Predicate,
    pub body: Vec<BodyExpr>,
}

#[derive(Debug, Clone)]
pub struct Predicate {
    pub name: String,
    pub args: Vec<TypedArg>,
}

#[derive(Debug, Clone)]
pub struct TypedArg {
    pub name: String,
    pub type_ann: Option<TypeExpr>,
}

// === Abduce ===

#[derive(Debug, Clone)]
pub struct AbduceDecl {
    pub head: Predicate,
    pub body: Vec<BodyExpr>,
}

// === Constraint ===

#[derive(Debug, Clone)]
pub struct ConstraintDecl {
    pub head: Predicate,
    pub body: Vec<BodyExpr>,
}

// === Functions ===

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<TypedArg>,
    pub return_type: Option<TypeExpr>,
    pub body: Vec<FnBodyExpr>,
}

#[derive(Debug, Clone)]
pub enum FnBodyExpr {
    /// A pipe expression: expr |> func |> func
    Pipe(Vec<Expr>),
    /// A let binding: let x = expr
    Let(String, Expr),
    /// A plain expression
    Expr(Expr),
}

// === Expressions ===

#[derive(Debug, Clone)]
pub enum Expr {
    /// Variable or identifier
    Ident(String),
    /// String literal
    StringLit(String),
    /// Integer literal
    IntLit(i64),
    /// Float literal
    FloatLit(f64),
    /// Function call: name(args...)
    Call(String, Vec<Expr>),
    /// Binary operation: left op right
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    /// Lambda: (params) => body
    Lambda(Vec<String>, Box<Expr>),
    /// Field access: expr.field
    FieldAccess(Box<Expr>, String),
    /// Pipe: left |> right
    Pipe(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Eq,     // ==
    Neq,    // !=
    Lt,     // <
    Gt,     // >
    Le,     // <=
    Ge,     // >=
    Add,    // +
    Sub,    // -
    Mul,    // *
    Div,    // /
    And,    // and / &&
    Or,     // or / ||
}

// === Body expressions (for rules, constraints, abduce) ===

#[derive(Debug, Clone)]
pub enum BodyExpr {
    /// Predicate call: pred(args...)
    PredicateCall(String, Vec<Expr>),
    /// Negation: not expr
    Not(Box<BodyExpr>),
    /// Let binding: let x = expr
    Let(String, Expr),
    /// Boolean expression (comparisons, etc.)
    Condition(Expr),
    /// Explain clause (for abduce)
    Explain(String),
    /// Import within scope
    ScopedImport(Import),
}

// === Types ===

#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// Simple named type: String, Int, Path, etc.
    Named(String),
    /// Parameterized type: List(T), Map(K, V)
    Parameterized(String, Vec<TypeExpr>),
    /// Constrained type: T where condition
    Constrained(Box<TypeExpr>, Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct TypeAlias {
    pub name: String,
    pub definition: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct DataDef {
    pub name: String,
    pub fields: Vec<DataField>,
}

#[derive(Debug, Clone)]
pub struct DataField {
    pub name: String,
    pub type_expr: TypeExpr,
    pub constraint: Option<Expr>,
}

// === Relations & Instances ===

#[derive(Debug, Clone)]
pub struct RelationDecl {
    pub name: String,
    pub params: Vec<TypedArg>,
    pub members: Vec<RelationMember>,
}

#[derive(Debug, Clone)]
pub enum RelationMember {
    Fn(FnDecl),
    NestedInstance(InstanceDecl),
}

#[derive(Debug, Clone)]
pub struct InstanceDecl {
    pub relation_name: String,
    pub type_args: Vec<TypeExpr>,
    pub where_clause: Option<Expr>,
    pub members: Vec<InstanceMember>,
}

#[derive(Debug, Clone)]
pub enum InstanceMember {
    Fn(FnDecl),
    NestedInstance(InstanceDecl),
}
