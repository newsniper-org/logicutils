use super::ast::*;
use super::lexer::{self, Located, Token};

#[derive(Debug, Clone, thiserror::Error)]
#[error("parse error at line {line}: {msg}")]
pub struct ParseError {
    pub line: usize,
    pub msg: String,
}

struct Parser {
    tokens: Vec<Located>,
    pos: usize,
}

/// Parse KB source text into a Module.
pub fn parse(input: &str) -> Result<Module, ParseError> {
    let tokens = lexer::tokenize(input).map_err(|e| ParseError {
        line: e.line,
        msg: e.msg,
    })?;
    let mut parser = Parser { tokens, pos: 0 };
    parser.parse_module()
}

impl Parser {
    fn peek(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .map(|l| &l.token)
            .unwrap_or(&Token::Eof)
    }

    fn current_line(&self) -> usize {
        self.tokens.get(self.pos).map(|l| l.line).unwrap_or(0)
    }

    fn advance(&mut self) -> &Token {
        let tok = self.tokens.get(self.pos).map(|l| &l.token).unwrap_or(&Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        let actual = self.peek().clone();
        if &actual == expected {
            self.advance();
            Ok(())
        } else {
            Err(ParseError {
                line: self.current_line(),
                msg: format!("expected {expected:?}, got {actual:?}"),
            })
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Token::Newline) {
            self.advance();
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance();
                Ok(name)
            }
            // Contextual keywords: allowed as identifiers in expression/argument positions
            Token::Data => { self.advance(); Ok("data".into()) }
            Token::Type => { self.advance(); Ok("type".into()) }
            Token::As => { self.advance(); Ok("as".into()) }
            other => Err(ParseError {
                line: self.current_line(),
                msg: format!("expected identifier, got {other:?}"),
            }),
        }
    }

    // === Module ===

    fn parse_module(&mut self) -> Result<Module, ParseError> {
        let mut items = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek(), Token::Eof) {
            let item = self.parse_item()?;
            items.push(item);
            self.skip_newlines();
        }
        Ok(Module { items })
    }

    fn parse_item(&mut self) -> Result<Item, ParseError> {
        match self.peek().clone() {
            Token::Import => self.parse_import().map(Item::Import),
            Token::Export => self.parse_export().map(Item::Export),
            Token::Fact => self.parse_fact_block().map(Item::Fact),
            Token::Rule => self.parse_rule().map(Item::Rule),
            Token::Abduce => self.parse_abduce().map(Item::Abduce),
            Token::Constraint => self.parse_constraint().map(Item::Constraint),
            Token::Fn => self.parse_fn().map(Item::Fn),
            Token::Type => self.parse_type_alias().map(Item::TypeAlias),
            Token::Data => self.parse_data().map(Item::DataDef),
            Token::Relation => self.parse_relation().map(Item::Relation),
            Token::Instance => self.parse_instance().map(Item::Instance),
            other => Err(ParseError {
                line: self.current_line(),
                msg: format!("unexpected token at top level: {other:?}"),
            }),
        }
    }

    // === Import / Export ===

    fn parse_import(&mut self) -> Result<Import, ParseError> {
        self.expect(&Token::Import)?;
        let path = self.parse_dotted_path()?;

        let mut names = None;
        let mut alias = None;

        if matches!(self.peek(), Token::LParen) {
            self.advance();
            let mut ns = Vec::new();
            loop {
                ns.push(self.expect_ident()?);
                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(&Token::RParen)?;
            names = Some(ns);
        } else if matches!(self.peek(), Token::As) {
            self.advance();
            alias = Some(self.expect_ident()?);
        }

        Ok(Import { path, names, alias })
    }

    fn parse_export(&mut self) -> Result<Export, ParseError> {
        self.expect(&Token::Export)?;
        let path = self.parse_dotted_path()?;
        Ok(Export { path })
    }

    fn parse_dotted_path(&mut self) -> Result<Vec<String>, ParseError> {
        let mut path = vec![self.expect_ident()?];
        while matches!(self.peek(), Token::Dot) {
            self.advance();
            path.push(self.expect_ident()?);
        }
        Ok(path)
    }

    // === Fact block ===

    fn parse_fact_block(&mut self) -> Result<FactBlock, ParseError> {
        self.expect(&Token::Fact)?;
        let name = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        self.skip_newlines();
        self.expect(&Token::Indent)?;

        let mut entries = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            let target = self.expect_ident()?;
            self.expect(&Token::Arrow)?;
            let dep = self.expect_ident()?;
            entries.push(FactEntry { target, dep });
        }

        if matches!(self.peek(), Token::Dedent) {
            self.advance();
        }
        Ok(FactBlock { name, entries })
    }

    // === Rule ===

    fn parse_rule(&mut self) -> Result<RuleDecl, ParseError> {
        self.expect(&Token::Rule)?;
        let head = self.parse_predicate()?;
        self.expect(&Token::Colon)?;
        let body = self.parse_body_block()?;
        Ok(RuleDecl { head, body })
    }

    // === Abduce ===

    fn parse_abduce(&mut self) -> Result<AbduceDecl, ParseError> {
        self.expect(&Token::Abduce)?;
        let head = self.parse_predicate()?;
        self.expect(&Token::Colon)?;
        let body = self.parse_body_block()?;
        Ok(AbduceDecl { head, body })
    }

    // === Constraint ===

    fn parse_constraint(&mut self) -> Result<ConstraintDecl, ParseError> {
        self.expect(&Token::Constraint)?;
        let head = self.parse_predicate()?;
        self.expect(&Token::Colon)?;
        let body = self.parse_body_block()?;
        Ok(ConstraintDecl { head, body })
    }

    // === Predicate ===

    fn parse_predicate(&mut self) -> Result<Predicate, ParseError> {
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let args = self.parse_typed_arg_list()?;
        self.expect(&Token::RParen)?;
        Ok(Predicate { name, args })
    }

    fn parse_typed_arg_list(&mut self) -> Result<Vec<TypedArg>, ParseError> {
        let mut args = Vec::new();
        if matches!(self.peek(), Token::RParen) {
            return Ok(args);
        }
        loop {
            let name = self.expect_ident()?;
            let type_ann = if matches!(self.peek(), Token::Colon) {
                self.advance();
                Some(self.parse_type_expr()?)
            } else {
                None
            };
            args.push(TypedArg { name, type_ann });
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(args)
    }

    // === Body block (for rule, abduce, constraint) ===

    fn parse_body_block(&mut self) -> Result<Vec<BodyExpr>, ParseError> {
        self.skip_newlines();
        self.expect(&Token::Indent)?;
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            body.push(self.parse_body_expr()?);
        }
        if matches!(self.peek(), Token::Dedent) {
            self.advance();
        }
        Ok(body)
    }

    fn parse_body_expr(&mut self) -> Result<BodyExpr, ParseError> {
        match self.peek().clone() {
            Token::Not => {
                self.advance();
                let inner = self.parse_body_expr()?;
                Ok(BodyExpr::Not(Box::new(inner)))
            }
            Token::Let => {
                self.advance();
                let name = self.expect_ident()?;
                self.expect(&Token::Assign)?;
                let expr = self.parse_expr()?;
                Ok(BodyExpr::Let(name, expr))
            }
            Token::Explain => {
                self.advance();
                match self.peek().clone() {
                    Token::StringLit(s) => {
                        self.advance();
                        Ok(BodyExpr::Explain(s))
                    }
                    _ => Err(ParseError {
                        line: self.current_line(),
                        msg: "expected string after 'explain'".into(),
                    }),
                }
            }
            Token::Import => {
                let imp = self.parse_import()?;
                Ok(BodyExpr::ScopedImport(imp))
            }
            Token::Ident(_) => {
                // Could be predicate call or condition
                let name = self.expect_ident()?;
                if matches!(self.peek(), Token::LParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Token::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek(), Token::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(BodyExpr::PredicateCall(name, args))
                } else {
                    // It's a condition expression starting with an ident
                    let left = Expr::Ident(name);
                    let expr = self.parse_expr_rest(left)?;
                    Ok(BodyExpr::Condition(expr))
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                Ok(BodyExpr::Condition(expr))
            }
        }
    }

    // === Function ===

    fn parse_fn(&mut self) -> Result<FnDecl, ParseError> {
        self.expect(&Token::Fn)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let params = self.parse_typed_arg_list()?;
        self.expect(&Token::RParen)?;

        let return_type = if matches!(self.peek(), Token::RightArrow) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        self.expect(&Token::Colon)?;
        self.skip_newlines();

        let body = if matches!(self.peek(), Token::Indent) {
            self.advance();
            let mut body = Vec::new();
            loop {
                self.skip_newlines();
                if matches!(self.peek(), Token::Dedent | Token::Eof) {
                    break;
                }
                body.push(self.parse_fn_body_expr()?);
            }
            if matches!(self.peek(), Token::Dedent) {
                self.advance();
            }
            body
        } else {
            // Single-line body
            vec![self.parse_fn_body_expr()?]
        };

        Ok(FnDecl {
            name,
            params,
            return_type,
            body,
        })
    }

    fn parse_fn_body_expr(&mut self) -> Result<FnBodyExpr, ParseError> {
        if matches!(self.peek(), Token::Let) {
            self.advance();
            let name = self.expect_ident()?;
            self.expect(&Token::Assign)?;
            let expr = self.parse_expr()?;
            Ok(FnBodyExpr::Let(name, expr))
        } else {
            let expr = self.parse_expr()?;
            Ok(FnBodyExpr::Expr(expr))
        }
    }

    // === Type ===

    fn parse_type_alias(&mut self) -> Result<TypeAlias, ParseError> {
        self.expect(&Token::Type)?;
        let name = self.expect_ident()?;
        self.expect(&Token::Assign)?;
        let definition = self.parse_type_expr()?;
        Ok(TypeAlias { name, definition })
    }

    fn parse_type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        let name = self.expect_ident()?;

        let base = if matches!(self.peek(), Token::LParen) {
            self.advance();
            let mut params = Vec::new();
            if !matches!(self.peek(), Token::RParen) {
                loop {
                    params.push(self.parse_type_expr()?);
                    if matches!(self.peek(), Token::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
            self.expect(&Token::RParen)?;
            TypeExpr::Parameterized(name, params)
        } else {
            TypeExpr::Named(name)
        };

        if matches!(self.peek(), Token::Where) {
            self.advance();
            let constraint = self.parse_expr()?;
            Ok(TypeExpr::Constrained(Box::new(base), Box::new(constraint)))
        } else {
            Ok(base)
        }
    }

    // === Data ===

    fn parse_data(&mut self) -> Result<DataDef, ParseError> {
        self.expect(&Token::Data)?;
        let name = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        self.skip_newlines();
        self.expect(&Token::Indent)?;

        let mut fields = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            let field_name = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let type_expr = self.parse_type_expr()?;
            let constraint = if matches!(self.peek(), Token::Where) {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            fields.push(DataField {
                name: field_name,
                type_expr,
                constraint,
            });
        }
        if matches!(self.peek(), Token::Dedent) {
            self.advance();
        }
        Ok(DataDef { name, fields })
    }

    // === Relation ===

    fn parse_relation(&mut self) -> Result<RelationDecl, ParseError> {
        self.expect(&Token::Relation)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let params = self.parse_typed_arg_list()?;
        self.expect(&Token::RParen)?;
        self.expect(&Token::Colon)?;
        self.skip_newlines();
        self.expect(&Token::Indent)?;

        let mut members = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            match self.peek().clone() {
                Token::Fn => members.push(RelationMember::Fn(self.parse_fn()?)),
                Token::Instance => {
                    members.push(RelationMember::NestedInstance(self.parse_instance()?))
                }
                other => {
                    return Err(ParseError {
                        line: self.current_line(),
                        msg: format!("unexpected in relation body: {other:?}"),
                    })
                }
            }
        }
        if matches!(self.peek(), Token::Dedent) {
            self.advance();
        }
        Ok(RelationDecl {
            name,
            params,
            members,
        })
    }

    // === Instance ===

    fn parse_instance(&mut self) -> Result<InstanceDecl, ParseError> {
        self.expect(&Token::Instance)?;
        let relation_name = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let mut type_args = Vec::new();
        if !matches!(self.peek(), Token::RParen) {
            loop {
                type_args.push(self.parse_type_expr()?);
                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(&Token::RParen)?;

        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };

        self.expect(&Token::Colon)?;
        self.skip_newlines();
        self.expect(&Token::Indent)?;

        let mut members = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Dedent | Token::Eof) {
                break;
            }
            match self.peek().clone() {
                Token::Fn => members.push(InstanceMember::Fn(self.parse_fn()?)),
                Token::Instance => {
                    members.push(InstanceMember::NestedInstance(self.parse_instance()?))
                }
                other => {
                    return Err(ParseError {
                        line: self.current_line(),
                        msg: format!("unexpected in instance body: {other:?}"),
                    })
                }
            }
        }
        if matches!(self.peek(), Token::Dedent) {
            self.advance();
        }
        Ok(InstanceDecl {
            relation_name,
            type_args,
            where_clause,
            members,
        })
    }

    // === Expressions ===

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_primary()?;
        self.parse_expr_rest(left)
    }

    fn parse_expr_rest(&mut self, left: Expr) -> Result<Expr, ParseError> {
        match self.peek() {
            Token::Pipe => {
                self.advance();
                let right = self.parse_primary()?;
                let pipe = Expr::Pipe(Box::new(left), Box::new(right));
                self.parse_expr_rest(pipe)
            }
            Token::Eq => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Eq, Box::new(right)))
            }
            Token::Neq => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Neq, Box::new(right)))
            }
            Token::Lt => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Lt, Box::new(right)))
            }
            Token::Gt => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Gt, Box::new(right)))
            }
            Token::Le => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Le, Box::new(right)))
            }
            Token::Ge => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Ge, Box::new(right)))
            }
            Token::Plus => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Add, Box::new(right)))
            }
            Token::Minus => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Sub, Box::new(right)))
            }
            Token::Star => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Mul, Box::new(right)))
            }
            Token::Slash => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Div, Box::new(right)))
            }
            Token::And => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::And, Box::new(right)))
            }
            Token::Or => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::BinOp(Box::new(left), BinOp::Or, Box::new(right)))
            }
            Token::Dot => {
                self.advance();
                let field = self.expect_ident()?;
                let access = Expr::FieldAccess(Box::new(left), field);
                self.parse_expr_rest(access)
            }
            _ => Ok(left),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Token::IntLit(n) => {
                self.advance();
                Ok(Expr::IntLit(n))
            }
            Token::FloatLit(f) => {
                self.advance();
                Ok(Expr::FloatLit(f))
            }
            Token::StringLit(s) => {
                self.advance();
                Ok(Expr::StringLit(s))
            }
            // Contextual keywords as identifiers in expression positions
            Token::Data | Token::Type | Token::As => {
                let name = self.expect_ident()?;
                if matches!(self.peek(), Token::LParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Token::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek(), Token::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Token::Ident(name) => {
                self.advance();
                if matches!(self.peek(), Token::LParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Token::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek(), Token::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Token::LParen => {
                self.advance();
                // Could be grouping or lambda params
                // Try parsing as expression first
                let expr = self.parse_expr()?;
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    // Check for lambda: ) =>
                    if matches!(self.peek(), Token::FatArrow) {
                        // Backtrack not possible easily; treat as grouping for now
                        Ok(expr)
                    } else {
                        Ok(expr)
                    }
                } else if matches!(self.peek(), Token::Comma) {
                    // Lambda or tuple
                    // For now, parse as lambda: (a, b) => expr
                    let mut params = vec![match expr {
                        Expr::Ident(n) => n,
                        _ => {
                            return Err(ParseError {
                                line: self.current_line(),
                                msg: "expected identifier in lambda params".into(),
                            })
                        }
                    }];
                    while matches!(self.peek(), Token::Comma) {
                        self.advance();
                        params.push(self.expect_ident()?);
                    }
                    self.expect(&Token::RParen)?;
                    self.expect(&Token::FatArrow)?;
                    let body = self.parse_expr()?;
                    Ok(Expr::Lambda(params, Box::new(body)))
                } else {
                    Err(ParseError {
                        line: self.current_line(),
                        msg: format!("expected ')' or ',', got {:?}", self.peek()),
                    })
                }
            }
            other => Err(ParseError {
                line: self.current_line(),
                msg: format!("expected expression, got {other:?}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fact_block() {
        let input = "fact depends:\n  main_o <- main_c\n  main_o <- header_h\n";
        let module = parse(input).unwrap();
        assert_eq!(module.items.len(), 1);
        match &module.items[0] {
            Item::Fact(fb) => {
                assert_eq!(fb.name, "depends");
                assert_eq!(fb.entries.len(), 2);
                assert_eq!(fb.entries[0].target, "main_o");
                assert_eq!(fb.entries[0].dep, "main_c");
            }
            _ => panic!("expected fact block"),
        }
    }

    #[test]
    fn test_parse_rule() {
        let input = "rule stale(Target):\n  depends(Target, Dep)\n  newer(Dep, Target)\n";
        let module = parse(input).unwrap();
        assert_eq!(module.items.len(), 1);
        match &module.items[0] {
            Item::Rule(r) => {
                assert_eq!(r.head.name, "stale");
                assert_eq!(r.head.args.len(), 1);
                assert_eq!(r.body.len(), 2);
            }
            _ => panic!("expected rule"),
        }
    }

    #[test]
    fn test_parse_abduce() {
        let input =
            "abduce missing_source(File):\n  depends(Target, File)\n  not exists(File)\n  explain \"source may need generation\"\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Abduce(a) => {
                assert_eq!(a.head.name, "missing_source");
                assert_eq!(a.body.len(), 3);
                assert!(matches!(&a.body[1], BodyExpr::Not(_)));
                assert!(matches!(&a.body[2], BodyExpr::Explain(_)));
            }
            _ => panic!("expected abduce"),
        }
    }

    #[test]
    fn test_parse_constraint() {
        let input =
            "constraint valid_alignment(x: SampleId, y: Reference):\n  x != y\n  exists(\"{x}.fastq\")\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Constraint(c) => {
                assert_eq!(c.head.name, "valid_alignment");
                assert_eq!(c.head.args.len(), 2);
                assert!(c.head.args[0].type_ann.is_some());
                assert_eq!(c.body.len(), 2);
            }
            _ => panic!("expected constraint"),
        }
    }

    #[test]
    fn test_parse_fn() {
        let input = "fn stem(path: Path) -> String:\n  path |> split(\".\") |> head()\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Fn(f) => {
                assert_eq!(f.name, "stem");
                assert_eq!(f.params.len(), 1);
                assert!(f.return_type.is_some());
                assert_eq!(f.body.len(), 1);
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn test_parse_type_alias() {
        let input = "type SampleId = String where matches(\"[A-Z]{2}[0-9]+\")\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::TypeAlias(t) => {
                assert_eq!(t.name, "SampleId");
                assert!(matches!(&t.definition, TypeExpr::Constrained(_, _)));
            }
            _ => panic!("expected type alias"),
        }
    }

    #[test]
    fn test_parse_data() {
        let input = "data AlignResult:\n  bam_path: Path\n  quality: Float\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::DataDef(d) => {
                assert_eq!(d.name, "AlignResult");
                assert_eq!(d.fields.len(), 2);
                assert_eq!(d.fields[0].name, "bam_path");
            }
            _ => panic!("expected data"),
        }
    }

    #[test]
    fn test_parse_relation() {
        let input = "relation Processable(Input, Output, Engine):\n  fn process(input: Input, engine: Engine) -> Output:\n    exec(input)\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Relation(r) => {
                assert_eq!(r.name, "Processable");
                assert_eq!(r.params.len(), 3);
                assert_eq!(r.members.len(), 1);
                assert!(matches!(&r.members[0], RelationMember::Fn(_)));
            }
            _ => panic!("expected relation"),
        }
    }

    #[test]
    fn test_parse_instance() {
        let input = "instance Processable(FastQ, BAM, SLURM):\n  fn process(fq, engine):\n    exec(\"sbatch align.sh\")\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Instance(inst) => {
                assert_eq!(inst.relation_name, "Processable");
                assert_eq!(inst.type_args.len(), 3);
                assert!(inst.where_clause.is_none());
                assert_eq!(inst.members.len(), 1);
            }
            _ => panic!("expected instance"),
        }
    }

    #[test]
    fn test_parse_instance_with_where() {
        let input = "instance Batchable(Input, Output) where Engine == GPU:\n  fn batch_size(input):\n    estimate_vram(input)\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Instance(inst) => {
                assert_eq!(inst.relation_name, "Batchable");
                assert!(inst.where_clause.is_some());
            }
            _ => panic!("expected instance"),
        }
    }

    #[test]
    fn test_parse_nested_instance() {
        let input = "\
instance Processable(Dataset, Model, GPU):
  fn process(data, engine):
    train(data)
  instance Batchable(Dataset, Model) where Engine == GPU:
    fn batch_size(input):
      estimate_vram(input)
    instance Shardable(Dataset) where Dataset == LargeDataset:
      fn shard_count(input):
        divide(input, max_shard_size)
";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Instance(inst) => {
                assert_eq!(inst.members.len(), 2); // fn + nested instance
                match &inst.members[1] {
                    InstanceMember::NestedInstance(nested) => {
                        assert_eq!(nested.relation_name, "Batchable");
                        assert_eq!(nested.members.len(), 2); // fn + nested
                        match &nested.members[1] {
                            InstanceMember::NestedInstance(deep) => {
                                assert_eq!(deep.relation_name, "Shardable");
                            }
                            _ => panic!("expected deeply nested instance"),
                        }
                    }
                    _ => panic!("expected nested instance"),
                }
            }
            _ => panic!("expected instance"),
        }
    }

    #[test]
    fn test_parse_import() {
        let input = "import bio.alignment (align, index)\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Import(imp) => {
                assert_eq!(imp.path, vec!["bio", "alignment"]);
                assert_eq!(imp.names.as_ref().unwrap(), &vec!["align", "index"]);
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn test_parse_import_alias() {
        let input = "import utils.paths as P\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Import(imp) => {
                assert_eq!(imp.alias.as_ref().unwrap(), "P");
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn test_parse_export() {
        let input = "export bio.alignment\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Export(exp) => {
                assert_eq!(exp.path, vec!["bio", "alignment"]);
            }
            _ => panic!("expected export"),
        }
    }

    #[test]
    fn test_parse_full_module() {
        let input = "\
import bio.alignment (align)

fact depends:
  main_o <- main_c
  main_o <- header_h

rule stale(Target):
  depends(Target, Dep)
  newer(Dep, Target)

fn stem(path) -> String:
  path |> split(\".\") |> head()

constraint valid(x, y):
  x != y
";
        let module = parse(input).unwrap();
        assert_eq!(module.items.len(), 5);
        assert!(matches!(&module.items[0], Item::Import(_)));
        assert!(matches!(&module.items[1], Item::Fact(_)));
        assert!(matches!(&module.items[2], Item::Rule(_)));
        assert!(matches!(&module.items[3], Item::Fn(_)));
        assert!(matches!(&module.items[4], Item::Constraint(_)));
    }

    #[test]
    fn test_parse_lambda() {
        let input = "fn apply(f, xs):\n  map((x, y) => f(x, y), xs)\n";
        let module = parse(input).unwrap();
        match &module.items[0] {
            Item::Fn(f) => {
                assert_eq!(f.name, "apply");
            }
            _ => panic!("expected fn"),
        }
    }
}
