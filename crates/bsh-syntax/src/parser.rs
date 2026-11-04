//! Recursive descent parser for the BeanShell / Java grammar.

use crate::ast::*;
use crate::error::SyntaxError;
use crate::lexer;
use crate::span::Span;
use crate::token::{FloatKind, Kw, Token, TokenKind};

pub fn parse(src: &str) -> Result<Program, SyntaxError> {
    let tokens = lexer::tokenize(src)?;
    Parser::new(tokens).parse_program()
}

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    last_span: Span,
    class_depth: usize,
    /// Inside try(...) the ';' before ')' is optional.
    resource_mode: bool,
}

type PResult<T> = Result<T, SyntaxError>;

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        Parser {
            toks,
            pos: 0,
            last_span: Span::default(),
            class_depth: 0,
            resource_mode: false,
        }
    }

    // ---------------------------------------------------------------- helpers

    fn peek(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn peek_at(&self, off: usize) -> &Token {
        let i = (self.pos + off).min(self.toks.len() - 1);
        &self.toks[i]
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == kind
    }

    fn at_kw(&self, kw: Kw) -> bool {
        matches!(self.peek_kind(), TokenKind::Kw(k) if *k == kw)
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos.min(self.toks.len() - 1)].clone();
        self.last_span = t.span;
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> PResult<Token> {
        if self.at(kind) {
            Ok(self.bump())
        } else {
            Err(self.err(format!(
                "Encountered {} but expected {}.",
                self.peek().kind.describe(),
                what
            )))
        }
    }

    fn err(&self, msg: impl Into<String>) -> SyntaxError {
        SyntaxError::new(msg, self.peek().span)
    }

    fn span_from(&self, start: Span) -> Span {
        start.merge(self.last_span)
    }

    fn expect_ident(&mut self) -> PResult<(String, Span)> {
        match self.peek_kind().clone() {
            TokenKind::Ident(s) => {
                let span = self.peek().span;
                self.bump();
                Ok((s, span))
            }
            other => Err(self.err(format!(
                "Encountered {} but expected an identifier.",
                other.describe()
            ))),
        }
    }

    // -------------------------------------------------------------- program

    pub fn parse_program(&mut self) -> PResult<Program> {
        let start = self.peek().span;
        let mut stmts = Vec::new();
        while !self.at(&TokenKind::Eof) {
            stmts.push(self.parse_statement()?);
        }
        Ok(Program {
            stmts,
            span: self.span_from(start),
        })
    }

    /// Parse a single expression (useful for the REPL).
    pub fn parse_expression(&mut self) -> PResult<Expr> {
        let e = self.parse_expr()?;
        Ok(e)
    }

    /// Kotlin-style string template: "$name" and "${expr}".
    fn parse_string_template(&mut self, text: String, start: Span) -> PResult<Expr> {
        const ESCAPED_DOLLAR: char = '\u{E000}';
        if !text.contains('$') {
            let plain = text.replace(ESCAPED_DOLLAR, "$");
            return Ok(Expr::new(ExprKind::Literal(Literal::Str(plain)), start));
        }
        let chars: Vec<char> = text.chars().collect();
        let mut parts: Vec<Expr> = Vec::new();
        let mut literal = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == ESCAPED_DOLLAR {
                literal.push('$');
                i += 1;
                continue;
            }
            if chars[i] != '$' {
                literal.push(chars[i]);
                i += 1;
                continue;
            }
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                if !literal.is_empty() {
                    parts.push(Expr::new(
                        ExprKind::Literal(Literal::Str(std::mem::take(&mut literal))),
                        start,
                    ));
                }
                let end = match find_template_end(&chars, i + 2) {
                    Some(end) => end,
                    None => return Err(self.err("unterminated string template expression")),
                };
                let expr_text: String = chars[i + 2..end].iter().collect();
                let tokens = lexer::tokenize(&expr_text)?;
                let mut nested = Parser::new(tokens);
                parts.push(nested.parse_expr()?);
                i = end + 1;
                continue;
            }
            if i + 1 < chars.len() && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_') {
                if !literal.is_empty() {
                    parts.push(Expr::new(
                        ExprKind::Literal(Literal::Str(std::mem::take(&mut literal))),
                        start,
                    ));
                }
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[i + 1..j].iter().collect();
                parts.push(Expr::new(ExprKind::Name(vec![name]), start));
                i = j;
                continue;
            }
            literal.push('$');
            i += 1;
        }
        if !literal.is_empty() {
            parts.push(Expr::new(ExprKind::Literal(Literal::Str(literal)), start));
        }
        let mut iter = parts.into_iter();
        let mut expr = match iter.next() {
            Some(first) => first,
            None => Expr::new(ExprKind::Literal(Literal::Str(String::new())), start),
        };
        for rhs in iter {
            expr = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    lhs: Box::new(expr),
                    rhs: Box::new(rhs),
                },
                start,
            );
        }
        Ok(expr)
    }

    // ----------------------------------------------------------- statements

    fn parse_statement(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        match self.peek_kind().clone() {
            TokenKind::Semi => {
                self.bump();
                Ok(Stmt::new(StmtKind::Empty, start))
            }
            TokenKind::LBrace => {
                self.bump();
                let mut stmts = Vec::new();
                while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                    stmts.push(self.parse_statement()?);
                }
                self.expect(&TokenKind::RBrace, "'}'")?;
                Ok(Stmt::new(StmtKind::Block(stmts), self.span_from(start)))
            }
            TokenKind::Kw(Kw::If) => self.parse_if(),
            TokenKind::Kw(Kw::While) => self.parse_while(),
            TokenKind::Kw(Kw::Do) => self.parse_do_while(),
            TokenKind::Kw(Kw::For) => self.parse_for(),
            TokenKind::Kw(Kw::Switch) => self.parse_switch(),
            TokenKind::Ident(name) if name == "yield" => {
                self.bump();
                let e = self.parse_expr()?;
                self.end_statement()?;
                Ok(Stmt::new(StmtKind::Yield(e), self.span_from(start)))
            }
            TokenKind::Kw(Kw::Try) => self.parse_try(),
            TokenKind::Kw(Kw::Throw) => {
                self.bump();
                let e = self.parse_expr()?;
                self.end_statement()?;
                Ok(Stmt::new(StmtKind::Throw(e), self.span_from(start)))
            }
            TokenKind::Kw(Kw::Return) => {
                self.bump();
                let e = if self.at(&TokenKind::Semi) || self.at_statement_end() {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.end_statement()?;
                Ok(Stmt::new(StmtKind::Return(e), self.span_from(start)))
            }
            TokenKind::Kw(Kw::Break) => {
                self.bump();
                let label = self.opt_label();
                self.end_statement()?;
                Ok(Stmt::new(StmtKind::Break(label), self.span_from(start)))
            }
            TokenKind::Kw(Kw::Continue) => {
                self.bump();
                let label = self.opt_label();
                self.end_statement()?;
                Ok(Stmt::new(StmtKind::Continue(label), self.span_from(start)))
            }
            TokenKind::Kw(Kw::Synchronized)
                if matches!(self.peek_at(1).kind, TokenKind::LParen) =>
            {
                self.bump();
                self.expect(&TokenKind::LParen, "'('")?;
                let lock = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "')'")?;
                let body = self.parse_statement()?;
                Ok(Stmt::new(
                    StmtKind::Synchronized {
                        lock,
                        body: Box::new(body),
                    },
                    self.span_from(start),
                ))
            }
            TokenKind::Kw(Kw::Class) | TokenKind::Kw(Kw::Interface) | TokenKind::Kw(Kw::Enum) => {
                let mods = Modifiers::default();
                let cd = self.parse_class_decl(mods, start)?;
                Ok(Stmt::new(StmtKind::Class(cd), self.span_from(start)))
            }
            TokenKind::Ident(name)
                if name == "record" && matches!(self.peek_at(1).kind, TokenKind::Ident(_)) =>
            {
                self.parse_record(Modifiers::default(), start)
            }
            TokenKind::Ident(name)
                if name == "sealed"
                    && matches!(
                        self.peek_at(1).kind,
                        TokenKind::Kw(Kw::Class)
                            | TokenKind::Kw(Kw::Interface)
                            | TokenKind::Kw(Kw::Enum)
                    ) =>
            {
                self.bump();
                let cd = self.parse_class_decl(Modifiers::default(), start)?;
                Ok(Stmt::new(StmtKind::Class(cd), self.span_from(start)))
            }
            TokenKind::Kw(Kw::Import) => self.parse_import(),
            TokenKind::Ident(name)
                if name == "assert"
                    && matches!(
                        self.peek_at(1).kind,
                        TokenKind::Ident(_)
                            | TokenKind::Int { .. }
                            | TokenKind::Float { .. }
                            | TokenKind::Str(_)
                            | TokenKind::CharStr { .. }
                            | TokenKind::Kw(Kw::True)
                            | TokenKind::Kw(Kw::False)
                            | TokenKind::Kw(Kw::Null)
                            | TokenKind::Not
                            | TokenKind::Minus
                            | TokenKind::Plus
                            | TokenKind::Tilde
                    ) =>
            {
                self.bump();
                let cond = self.parse_expr()?;
                let message = if self.at(&TokenKind::Colon) {
                    self.bump();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.end_statement()?;
                Ok(Stmt::new(
                    StmtKind::Assert { cond, message },
                    self.span_from(start),
                ))
            }
            TokenKind::Kw(Kw::Package) => {
                self.bump();
                let (name, _) = self.parse_qualified_name()?;
                self.end_statement()?;
                Ok(Stmt::new(
                    StmtKind::Package(name.join(".")),
                    self.span_from(start),
                ))
            }
            TokenKind::At => {
                self.skip_annotations();
                self.parse_statement()
            }
            TokenKind::Ident(_) if matches!(self.peek_at(1).kind, TokenKind::Colon) => {
                let (label, _) = self.expect_ident()?;
                self.bump(); // ':'
                let body = self.parse_statement()?;
                Ok(Stmt::new(
                    StmtKind::Labeled {
                        label,
                        body: Box::new(body),
                    },
                    self.span_from(start),
                ))
            }
            _ => {
                if self.looks_like_decl() {
                    let mods = self.parse_modifiers();
                    self.parse_declaration(mods, start)
                } else if matches!(self.peek_kind(), TokenKind::Ident(_))
                    && matches!(
                        self.peek_at(1).kind,
                        TokenKind::Kw(Kw::Class)
                            | TokenKind::Kw(Kw::Interface)
                            | TokenKind::Kw(Kw::Enum)
                    )
                {
                    // "Foo class" is never valid but keep the error readable.
                    Err(self.err("expected a statement"))
                } else {
                    let e = self.parse_expr()?;
                    self.end_statement()?;
                    Ok(Stmt::new(StmtKind::Expr(e), self.span_from(start)))
                }
            }
        }
    }

    /// Kotlin-style statement end: ';', a newline, '}' or EOF.
    fn at_statement_end(&self) -> bool {
        let next = self.peek();
        matches!(next.kind, TokenKind::RBrace | TokenKind::Eof)
            || next.span.line > self.last_span.line
    }

    fn end_statement(&mut self) -> PResult<()> {
        if self.eat(&TokenKind::Semi) {
            return Ok(());
        }
        if self.at_statement_end() {
            return Ok(());
        }
        Err(self.err(format!(
            "Encountered {} but expected ';' or a newline",
            self.peek().kind.describe()
        )))
    }

    fn opt_label(&mut self) -> Option<String> {
        if let TokenKind::Ident(s) = self.peek_kind().clone() {
            // In semicolon-free code "continue\nnextStatement = ..." must not
            // swallow the next statement as a label. A label has to be on the
            // same line as break/continue and end the statement.
            let line = self.peek().span.line;
            let next = self.peek_at(1);
            let followed_by_semicolon = matches!(next.kind, TokenKind::Semi);
            if line == self.last_span.line && (followed_by_semicolon || next.span.line > line) {
                self.bump();
                return Some(s);
            }
        }
        None
    }

    fn parse_if(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        self.bump();
        self.expect(&TokenKind::LParen, "'('")?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "')'")?;
        let then_branch = self.parse_statement()?;
        let else_branch = if self.at_kw(Kw::Else) {
            self.bump();
            Some(Box::new(self.parse_statement()?))
        } else {
            None
        };
        Ok(Stmt::new(
            StmtKind::If {
                cond,
                then_branch: Box::new(then_branch),
                else_branch,
            },
            self.span_from(start),
        ))
    }

    fn parse_while(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        self.bump();
        self.expect(&TokenKind::LParen, "'('")?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "')'")?;
        let body = self.parse_statement()?;
        Ok(Stmt::new(
            StmtKind::While {
                cond,
                body: Box::new(body),
            },
            self.span_from(start),
        ))
    }

    fn parse_do_while(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        self.bump();
        let body = self.parse_statement()?;
        if !self.at_kw(Kw::While) {
            return Err(self.err("Encountered {} but expected 'while'."));
        }
        self.bump();
        self.expect(&TokenKind::LParen, "'('")?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "')'")?;
        self.end_statement()?;
        Ok(Stmt::new(
            StmtKind::DoWhile {
                body: Box::new(body),
                cond,
            },
            self.span_from(start),
        ))
    }

    fn parse_for(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        self.bump();
        self.expect(&TokenKind::LParen, "'('")?;

        // Enhanced for:  [Type] name : expr
        if let Some((ty, name)) = self.try_parse_foreach_head()? {
            let iter = self.parse_expr()?;
            self.expect(&TokenKind::RParen, "')'")?;
            let body = self.parse_statement()?;
            return Ok(Stmt::new(
                StmtKind::ForEach {
                    ty,
                    name,
                    iter,
                    body: Box::new(body),
                },
                self.span_from(start),
            ));
        }

        let init = if self.at(&TokenKind::Semi) {
            self.bump();
            None
        } else if self.looks_like_decl() {
            let s = self.parse_declaration(Modifiers::default(), self.peek().span)?;
            Some(Box::new(s))
        } else {
            let first = self.parse_expr()?;
            let mut stmts = vec![Stmt::new(StmtKind::Expr(first), self.peek().span)];
            while self.eat(&TokenKind::Comma) {
                let e = self.parse_expr()?;
                stmts.push(Stmt::new(StmtKind::Expr(e), self.peek().span));
            }
            self.expect(&TokenKind::Semi, "';'")?;
            let s = if stmts.len() == 1 {
                stmts.pop().unwrap()
            } else {
                Stmt::new(StmtKind::Block(stmts), self.peek().span)
            };
            Some(Box::new(s))
        };

        let cond = if self.at(&TokenKind::Semi) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(&TokenKind::Semi, "';'")?;

        let mut update = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                update.push(self.parse_expr()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen, "')'")?;
        let body = self.parse_statement()?;
        Ok(Stmt::new(
            StmtKind::For {
                init,
                cond,
                update,
                body: Box::new(body),
            },
            self.span_from(start),
        ))
    }

    /// Lookahead for the enhanced for header; restores the position on failure.
    fn try_parse_foreach_head(&mut self) -> PResult<Option<(Option<TypeRef>, String)>> {
        let save = self.pos;
        let save_span = self.last_span;
        let result = self.parse_foreach_head();
        if result.is_err() {
            self.pos = save;
            self.last_span = save_span;
            return Ok(None);
        }
        result
    }

    fn parse_foreach_head(&mut self) -> PResult<Option<(Option<TypeRef>, String)>> {
        if self.at_kw(Kw::Final) {
            self.bump();
        }
        match self.peek_kind().clone() {
            TokenKind::Ident(name) if matches!(self.peek_at(1).kind, TokenKind::Colon) => {
                self.bump();
                self.bump();
                Ok(Some((None, name)))
            }
            _ => {
                if !self.looks_like_type_start() {
                    return Ok(None);
                }
                let ty = self.parse_type()?;
                let (name, _) = self.expect_ident()?;
                if !self.at(&TokenKind::Colon) {
                    return Err(self.err("expected ':' in enhanced for"));
                }
                self.bump();
                Ok(Some((Some(ty), name)))
            }
        }
    }

    fn parse_switch(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        self.bump();
        self.expect(&TokenKind::LParen, "'('")?;
        let selector = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "')'")?;
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let arm_start = self.peek().span;
            let mut pattern = None;
            let label = if self.at_kw(Kw::Case) {
                self.bump();
                if let Some(p) = self.try_parse_switch_pattern()? {
                    pattern = Some(p);
                    None
                } else {
                    Some(self.parse_expr()?)
                }
            } else if self.at_kw(Kw::Default) {
                self.bump();
                None
            } else {
                return Err(self.err(format!(
                    "Encountered {} but expected 'case' or 'default'.",
                    self.peek().kind.describe()
                )));
            };
            let arrow = self.eat(&TokenKind::Arrow);
            let mut stmts = Vec::new();
            if arrow {
                if self.eat(&TokenKind::LBrace) {
                    while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                        stmts.push(self.parse_statement()?);
                    }
                    self.expect(&TokenKind::RBrace, "'}'")?;
                } else {
                    stmts.push(self.parse_statement()?);
                }
            } else {
                self.expect(&TokenKind::Colon, "':'")?;
                while !self.at_kw(Kw::Case)
                    && !self.at_kw(Kw::Default)
                    && !self.at(&TokenKind::RBrace)
                    && !self.at(&TokenKind::Eof)
                {
                    stmts.push(self.parse_statement()?);
                }
            }
            arms.push(SwitchArm {
                label,
                pattern,
                stmts,
                arrow,
                span: self.span_from(arm_start),
            });
        }
        self.expect(&TokenKind::RBrace, "'}'")?;
        Ok(Stmt::new(
            StmtKind::Switch { selector, arms },
            self.span_from(start),
        ))
    }

    fn try_parse_switch_pattern(&mut self) -> PResult<Option<SwitchPattern>> {
        let save = self.pos;
        let save_span = self.last_span;
        let result = (|| -> PResult<Option<SwitchPattern>> {
            let ty = self.parse_type()?;
            let mut components = Vec::new();
            if self.eat(&TokenKind::LParen) {
                if !self.at(&TokenKind::RParen) {
                    loop {
                        let cty = self.parse_type()?;
                        let (cname, _) = self.expect_ident()?;
                        components.push((cty, cname));
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "')'")?;
            }
            let name = if components.is_empty() {
                match self.peek_kind().clone() {
                    TokenKind::Ident(name) if matches!(self.peek_at(1).kind, TokenKind::Arrow) => {
                        self.bump();
                        Some(name)
                    }
                    _ => None,
                }
            } else {
                None
            };
            if !matches!(self.peek_kind(), TokenKind::Arrow) {
                return Ok(None);
            }
            let span = ty.span;
            Ok(Some(SwitchPattern {
                ty,
                name,
                components,
                span,
            }))
        })();
        match result {
            Ok(Some(p)) => Ok(Some(p)),
            _ => {
                self.pos = save;
                self.last_span = save_span;
                Ok(None)
            }
        }
    }

    fn parse_switch_expr(&mut self) -> PResult<Expr> {
        let start = self.peek().span;
        self.bump();
        self.expect(&TokenKind::LParen, "'('")?;
        let selector = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "')'")?;
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let arm_start = self.peek().span;
            let mut pattern = None;
            let label = if self.at_kw(Kw::Case) {
                self.bump();
                if let Some(p) = self.try_parse_switch_pattern()? {
                    pattern = Some(p);
                    None
                } else {
                    Some(self.parse_expr()?)
                }
            } else if self.at_kw(Kw::Default) {
                self.bump();
                None
            } else {
                return Err(self.err(format!(
                    "Encountered {} but expected 'case' or 'default'.",
                    self.peek().kind.describe()
                )));
            };
            self.expect(&TokenKind::Arrow, "'->'")?;
            let body = if self.eat(&TokenKind::LBrace) {
                let mut stmts = Vec::new();
                while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                    stmts.push(self.parse_statement()?);
                }
                self.expect(&TokenKind::RBrace, "'}'")?;
                SwitchExprBody::Block(stmts)
            } else {
                let expr = self.parse_expr()?;
                self.eat(&TokenKind::Semi);
                SwitchExprBody::Expr(expr)
            };
            arms.push(SwitchExprArm {
                label,
                pattern,
                body,
                span: self.span_from(arm_start),
            });
        }
        self.expect(&TokenKind::RBrace, "'}'")?;
        Ok(Expr::new(
            ExprKind::SwitchExpr {
                selector: Box::new(selector),
                arms,
            },
            self.span_from(start),
        ))
    }

    fn parse_try(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        self.bump();
        // try-with-resources: parse the resource declarations and treat them as
        // leading declarations of the body block.
        let mut resources: Vec<Stmt> = Vec::new();
        if self.at(&TokenKind::LParen) {
            self.bump();
            while !self.at(&TokenKind::RParen) {
                if self.looks_like_decl() || self.looks_like_resource_decl() {
                    self.resource_mode = true;
                    let parsed = self.parse_declaration(Modifiers::default(), self.peek().span);
                    self.resource_mode = false;
                    let mut d = parsed?;
                    if let StmtKind::VarDecl(vd) = &mut d.kind {
                        vd.modifiers.is_final = true;
                    }
                    resources.push(d);
                } else {
                    let e = self.parse_expr()?;
                    resources.push(Stmt::new(StmtKind::Expr(e), self.peek().span));
                }
                if !self.eat(&TokenKind::Semi) {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "')'")?;
        }
        let body = self.parse_statement()?;
        let mut catches = Vec::new();
        while self.at_kw(Kw::Catch) {
            let cstart = self.peek().span;
            self.bump();
            self.expect(&TokenKind::LParen, "'('")?;
            let mut types = Vec::new();
            // Untyped catch parameter: catch ( e ) matches any throwable.
            let untyped = matches!(self.peek_kind(), TokenKind::Ident(_))
                && matches!(self.peek_at(1).kind, TokenKind::RParen);
            if !untyped {
                loop {
                    types.push(self.parse_type()?);
                    if !self.eat(&TokenKind::Pipe) {
                        break;
                    }
                }
            }
            let (name, _) = self.expect_ident()?;
            self.expect(&TokenKind::RParen, "')'")?;
            let cbody = self.parse_statement()?;
            catches.push(CatchClause {
                types,
                name,
                body: cbody,
                span: self.span_from(cstart),
            });
        }
        let finally = if self.at_kw(Kw::Finally) {
            self.bump();
            Some(Box::new(self.parse_statement()?))
        } else {
            None
        };
        if catches.is_empty() && finally.is_none() && resources.is_empty() {
            return Err(self.err("'try' without 'catch' or 'finally'"));
        }
        // try-with-resources -> nested try/finally. A resource's declaration
        // sits inside its own try body, so if a later initializer fails the
        // resources acquired before it are still closed.
        let mut inner = body;
        for resource in resources.iter().rev() {
            let close = self.resource_close_stmt(resource)?;
            let needs_declaration = match &resource.kind {
                StmtKind::VarDecl(_) => true,
                StmtKind::Expr(Expr {
                    kind: ExprKind::Name(_),
                    ..
                }) => false,
                _ => true,
            };
            let mut statements = Vec::new();
            if needs_declaration {
                statements.push(resource.clone());
            }
            let close_block = Stmt::new(StmtKind::Block(vec![close]), resource.span);
            statements.push(Stmt::new(
                StmtKind::Try {
                    body: Box::new(inner),
                    catches: Vec::new(),
                    finally: Some(Box::new(close_block)),
                },
                resource.span,
            ));
            inner = Stmt::new(StmtKind::Block(statements), resource.span);
        }
        let body = inner;
        Ok(Stmt::new(
            StmtKind::Try {
                body: Box::new(body),
                catches,
                finally,
            },
            self.span_from(start),
        ))
    }

    /// Qualified type followed by a variable and '=': java.io.Reader r = ...
    fn looks_like_resource_decl(&self) -> bool {
        if !matches!(self.peek_kind(), TokenKind::Ident(_)) {
            return false;
        }
        let mut index = 1usize;
        while matches!(self.peek_at(index).kind, TokenKind::Dot)
            && matches!(self.peek_at(index + 1).kind, TokenKind::Ident(_))
        {
            index += 2;
        }
        matches!(self.peek_at(index).kind, TokenKind::Ident(_))
            && matches!(self.peek_at(index + 1).kind, TokenKind::Assign)
    }

    /// Build "__close(name);" for a try-with-resources entry.
    fn resource_close_stmt(&self, resource: &Stmt) -> PResult<Stmt> {
        let name = match &resource.kind {
            StmtKind::VarDecl(vd) => vd.vars.first().map(|v| v.name.clone()),
            StmtKind::Expr(Expr {
                kind: ExprKind::Name(parts),
                ..
            }) if parts.len() == 1 => Some(parts[0].clone()),
            StmtKind::Expr(Expr {
                kind: ExprKind::Assign { lhs, .. },
                ..
            }) => match &lhs.kind {
                ExprKind::Name(parts) if parts.len() == 1 => Some(parts[0].clone()),
                _ => None,
            },
            _ => None,
        };
        let target = Expr::new(ExprKind::Name(vec!["__close".to_string()]), resource.span);
        let argument = match name {
            Some(name) => Expr::new(ExprKind::Name(vec![name]), resource.span),
            None => match &resource.kind {
                StmtKind::Expr(expr) => expr.clone(),
                _ => return Err(self.err("invalid try-with-resources entry")),
            },
        };
        let call = Expr::new(
            ExprKind::Call {
                target: Box::new(target),
                args: vec![argument],
            },
            resource.span,
        );
        Ok(Stmt::new(StmtKind::Expr(call), resource.span))
    }

    fn parse_import(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        self.bump();
        let is_static = self.at_kw(Kw::Static);
        if is_static {
            self.bump();
        }
        let (name, wildcard) = if self.at(&TokenKind::Star) {
            self.bump();
            (vec!["*".to_string()], true)
        } else {
            let (mut name, _) = self.parse_qualified_name()?;
            let wildcard =
                if self.at(&TokenKind::Dot) && matches!(self.peek_at(1).kind, TokenKind::Star) {
                    self.bump();
                    self.bump();
                    true
                } else {
                    false
                };
            let _ = &mut name;
            (name, wildcard)
        };
        self.end_statement()?;
        Ok(Stmt::new(
            StmtKind::Import(ImportDecl {
                name,
                is_static,
                wildcard,
                span: self.span_from(start),
            }),
            self.span_from(start),
        ))
    }

    fn parse_qualified_name(&mut self) -> PResult<(Vec<String>, Span)> {
        let start = self.peek().span;
        let mut parts = Vec::new();
        let (first, _) = self.expect_ident()?;
        parts.push(first);
        while self.at(&TokenKind::Dot) && matches!(self.peek_at(1).kind, TokenKind::Ident(_)) {
            self.bump();
            let (p, _) = self.expect_ident()?;
            parts.push(p);
        }
        Ok((parts, self.span_from(start)))
    }

    // ---------------------------------------------------------- declarations

    fn skip_annotations(&mut self) {
        while self.at(&TokenKind::At) {
            self.bump();
            let _ = self.parse_qualified_name();
            if self.at(&TokenKind::LParen) {
                let mut depth = 0;
                loop {
                    match self.peek_kind() {
                        TokenKind::LParen => {
                            depth += 1;
                            self.bump();
                        }
                        TokenKind::RParen => {
                            depth -= 1;
                            self.bump();
                            if depth == 0 {
                                break;
                            }
                        }
                        TokenKind::Eof => break,
                        _ => {
                            self.bump();
                        }
                    }
                }
            }
        }
    }

    fn parse_modifiers(&mut self) -> Modifiers {
        let mut m = Modifiers::default();
        loop {
            match self.peek_kind() {
                TokenKind::Kw(Kw::Public) => m.is_public = true,
                TokenKind::Kw(Kw::Private) => m.is_private = true,
                TokenKind::Kw(Kw::Protected) => m.is_protected = true,
                TokenKind::Kw(Kw::Static) => m.is_static = true,
                TokenKind::Kw(Kw::Final) => m.is_final = true,
                TokenKind::Kw(Kw::Abstract) => m.is_abstract = true,
                TokenKind::Kw(Kw::Synchronized) => m.is_synchronized = true,
                TokenKind::Kw(Kw::Transient) => m.is_transient = true,
                TokenKind::Kw(Kw::Volatile) => m.is_volatile = true,
                TokenKind::Kw(Kw::Native) => m.is_native = true,
                TokenKind::Kw(Kw::Strictfp) => m.is_strictfp = true,
                TokenKind::Kw(Kw::Default) => m.is_default = true,
                _ => break,
            }
            self.bump();
        }
        m
    }

    fn looks_like_type_start(&self) -> bool {
        match self.peek_kind() {
            TokenKind::Kw(k) => {
                let k = *k;
                matches!(
                    k,
                    Kw::Boolean
                        | Kw::Byte
                        | Kw::Char
                        | Kw::Short
                        | Kw::Int
                        | Kw::Long
                        | Kw::Float
                        | Kw::Double
                        | Kw::Void
                )
            }
            TokenKind::Ident(_) => true,
            _ => false,
        }
    }

    /// Speculative declaration detection. Returns true when the tokens at the
    /// current position can only be a declaration.
    fn looks_like_decl(&self) -> bool {
        let mut i = self.pos;
        // Annotations and modifiers.
        while matches!(self.toks[i].kind, TokenKind::At) {
            i += 1;
            // qualified name
            if matches!(self.toks[i].kind, TokenKind::Ident(_)) {
                i += 1;
                while matches!(self.toks[i].kind, TokenKind::Dot)
                    && matches!(self.toks[i + 1].kind, TokenKind::Ident(_))
                {
                    i += 2;
                }
            }
            if matches!(self.toks[i].kind, TokenKind::LParen) {
                let mut depth = 0usize;
                loop {
                    match self.toks[i].kind {
                        TokenKind::LParen => {
                            depth += 1;
                            i += 1;
                        }
                        TokenKind::RParen => {
                            depth -= 1;
                            i += 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        TokenKind::Eof => return false,
                        _ => i += 1,
                    }
                }
            }
        }
        let mut saw_modifier = false;
        loop {
            match self.toks[i].kind {
                TokenKind::Kw(Kw::Public)
                | TokenKind::Kw(Kw::Private)
                | TokenKind::Kw(Kw::Protected)
                | TokenKind::Kw(Kw::Static)
                | TokenKind::Kw(Kw::Final)
                | TokenKind::Kw(Kw::Abstract)
                | TokenKind::Kw(Kw::Strictfp)
                | TokenKind::Kw(Kw::Native)
                | TokenKind::Kw(Kw::Transient)
                | TokenKind::Kw(Kw::Default)
                | TokenKind::Kw(Kw::Volatile) => {
                    saw_modifier = true;
                    i += 1;
                }
                // "synchronized" is a statement keyword only when directly
                // followed by '('; the statement parser checks that first.
                TokenKind::Kw(Kw::Synchronized) => {
                    saw_modifier = true;
                    i += 1;
                }
                _ => break,
            }
        }
        if saw_modifier {
            return true;
        }
        match &self.toks[i].kind {
            TokenKind::Kw(Kw::Class) | TokenKind::Kw(Kw::Interface) | TokenKind::Kw(Kw::Enum) => {
                true
            }
            TokenKind::Kw(k) => {
                PrimKind::from_kw(*k).is_some()
                    && !matches!(self.toks[i + 1].kind, TokenKind::Assign | TokenKind::Dot)
            }
            TokenKind::Ident(_) => match scan_type_end(&self.toks, i) {
                Some(j) => match &self.toks[j].kind {
                    // "Type name" or "Type<...> name"
                    TokenKind::Ident(_) => true,
                    // "methodName(" -- declaration only when a body follows
                    TokenKind::LParen => match matching_paren(&self.toks, j) {
                        Some(k) => {
                            matches!(self.toks[k + 1].kind, TokenKind::LBrace)
                                || matches!(self.toks[k + 1].kind, TokenKind::Kw(Kw::Throws))
                        }
                        None => false,
                    },
                    _ => false,
                },
                None => false,
            },
            _ => false,
        }
    }

    fn parse_declaration(&mut self, mods: Modifiers, start: Span) -> PResult<Stmt> {
        if matches!(
            self.peek_kind(),
            TokenKind::Kw(Kw::Class) | TokenKind::Kw(Kw::Interface) | TokenKind::Kw(Kw::Enum)
        ) {
            let cd = self.parse_class_decl(mods, start)?;
            return Ok(Stmt::new(StmtKind::Class(cd), self.span_from(start)));
        }
        // Generic method type parameters: <T> T foo(...)
        if self.at(&TokenKind::Lt) {
            if let Some(j) = skip_generics(&self.toks, self.pos) {
                self.pos = j;
            }
        }
        // Untyped method declaration: foo(...) { ... }
        if let TokenKind::Ident(_) = self.peek_kind() {
            if matches!(self.peek_at(1).kind, TokenKind::LParen) {
                let (name, _) = self.expect_ident()?;
                let m = self.parse_method_tail(mods, None, name, start)?;
                return Ok(Stmt::new(StmtKind::Method(m), self.span_from(start)));
            }
        }
        let ty = self.parse_type()?;
        let (name, name_span) = self.expect_ident()?;
        if self.at(&TokenKind::LParen) || self.at_kw(Kw::Throws) {
            let m = self.parse_method_tail(mods, Some(ty), name, start)?;
            return Ok(Stmt::new(StmtKind::Method(m), self.span_from(start)));
        }
        let mut vars = Vec::new();
        let mut vd = VarDeclarator {
            name,
            dims: 0,
            init: None,
            span: name_span,
        };
        self.parse_declarator_rest(&mut vd)?;
        vars.push(vd);
        while self.eat(&TokenKind::Comma) {
            let (name, span) = self.expect_ident()?;
            let mut vd = VarDeclarator {
                name,
                dims: 0,
                init: None,
                span,
            };
            self.parse_declarator_rest(&mut vd)?;
            vars.push(vd);
        }
        if !self.resource_mode {
            self.end_statement()?;
        }
        Ok(Stmt::new(
            StmtKind::VarDecl(VarDecl {
                modifiers: mods,
                ty: Some(ty),
                vars,
                span: self.span_from(start),
            }),
            self.span_from(start),
        ))
    }

    fn parse_declarator_rest(&mut self, vd: &mut VarDeclarator) -> PResult<()> {
        while self.at(&TokenKind::LBracket) && matches!(self.peek_at(1).kind, TokenKind::RBracket) {
            self.bump();
            self.bump();
            vd.dims += 1;
        }
        if self.eat(&TokenKind::Assign) {
            vd.init = Some(self.parse_array_initializer_or_expr()?);
        }
        Ok(())
    }

    fn looks_like_param_type(&self) -> bool {
        match scan_type_end(&self.toks, self.pos) {
            Some(j) if j > self.pos => {
                matches!(self.toks[j].kind, TokenKind::Ident(_) | TokenKind::Ellipsis)
            }
            _ => false,
        }
    }

    fn parse_method_tail(
        &mut self,
        mods: Modifiers,
        return_type: Option<TypeRef>,
        name: String,
        start: Span,
    ) -> PResult<MethodDecl> {
        self.expect(&TokenKind::LParen, "'('")?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                if self.at_kw(Kw::Final) {
                    self.bump();
                }
                let pty = if self.looks_like_param_type() {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let varargs = if self.at(&TokenKind::Ellipsis) {
                    self.bump();
                    true
                } else {
                    false
                };
                let (pname, _) = self.expect_ident()?;
                while self.eat(&TokenKind::LBracket) {
                    self.expect(&TokenKind::RBracket, "']'")?;
                }
                params.push(Param {
                    ty: pty,
                    name: pname,
                    varargs,
                    span: self.last_span,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen, "')'")?;
        let mut throws = Vec::new();
        if self.at_kw(Kw::Throws) {
            self.bump();
            loop {
                throws.push(self.parse_type()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        let body = if self.at(&TokenKind::LBrace) {
            Some(Box::new(self.parse_statement()?))
        } else {
            self.expect(&TokenKind::Semi, "';'")?;
            None
        };
        Ok(MethodDecl {
            modifiers: mods,
            return_type,
            name,
            params,
            throws,
            body,
            is_ctor: false,
            span: self.span_from(start),
        })
    }

    fn parse_class_decl(&mut self, modifiers: Modifiers, start: Span) -> PResult<ClassDecl> {
        let kind = match self.peek_kind() {
            TokenKind::Kw(Kw::Class) => ClassKind::Class,
            TokenKind::Kw(Kw::Interface) => ClassKind::Interface,
            TokenKind::Kw(Kw::Enum) => ClassKind::Enum,
            _ => return Err(self.err("expected 'class', 'interface' or 'enum'")),
        };
        self.bump();
        let (name, _) = self.expect_ident()?;
        // Generic type parameters are erased.
        if self.at(&TokenKind::Lt) {
            if let Some(j) = skip_generics(&self.toks, self.pos) {
                self.pos = j;
            }
        }
        let mut extends = None;
        let mut implements = Vec::new();
        if self.at_kw(Kw::Extends) {
            self.bump();
            extends = Some(self.parse_type()?);
            if kind == ClassKind::Interface {
                while self.eat(&TokenKind::Comma) {
                    implements.push(self.parse_type()?);
                }
            }
        }
        if self.at_kw(Kw::Implements) {
            self.bump();
            loop {
                implements.push(self.parse_type()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        if matches!(self.peek_kind(), TokenKind::Ident(name) if name == "permits") {
            self.bump();
            loop {
                let _ = self.parse_type()?;
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::LBrace, "'{'")?;
        self.class_depth += 1;
        let mut enum_constants = Vec::new();
        if kind == ClassKind::Enum {
            loop {
                match self.peek_kind().clone() {
                    TokenKind::Ident(cname) => {
                        let cstart = self.peek().span;
                        self.bump();
                        let mut args = Vec::new();
                        if self.at(&TokenKind::LParen) {
                            args = self.parse_arguments()?;
                        }
                        let mut body = None;
                        if self.at(&TokenKind::LBrace) {
                            self.bump();
                            let mut members = Vec::new();
                            while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                                members.push(self.parse_class_member()?);
                            }
                            self.expect(&TokenKind::RBrace, "'}'")?;
                            body = Some(members);
                        }
                        enum_constants.push(EnumConstant {
                            name: cname,
                            args,
                            body,
                            span: self.span_from(cstart),
                        });
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            if self.at(&TokenKind::Semi) {
                self.bump();
            }
        }
        let mut members = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            if self.eat(&TokenKind::Semi) {
                continue;
            }
            members.push(self.parse_class_member()?);
        }
        self.expect(&TokenKind::RBrace, "'}'")?;
        self.class_depth -= 1;
        // Mark constructors: methods whose name equals the class name and that
        // have no declared return type.
        for m in members.iter_mut() {
            if let StmtKind::Method(md) = &mut m.kind {
                if md.return_type.is_none() && md.name == name {
                    md.is_ctor = true;
                }
            }
        }
        Ok(ClassDecl {
            modifiers,
            kind,
            name,
            extends,
            implements,
            enum_constants,
            members,
            span: self.span_from(start),
        })
    }

    /// Java 16 record: desugar to a final class with fields, a canonical
    /// constructor and accessors.
    fn parse_record(&mut self, mut modifiers: Modifiers, start: Span) -> PResult<Stmt> {
        modifiers.is_final = true;
        self.bump(); // record
        let (name, name_span) = self.expect_ident()?;
        if self.at(&TokenKind::Lt) {
            if let Some(j) = skip_generics(&self.toks, self.pos) {
                self.pos = j;
            }
        }
        self.expect(&TokenKind::LParen, "'('")?;
        let mut components: Vec<(TypeRef, String, Span)> = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                let ty = self.parse_type()?;
                let (cname, cspan) = self.expect_ident()?;
                components.push((ty, cname, cspan));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen, "')'")?;

        let mut implements = Vec::new();
        if self.at_kw(Kw::Implements) {
            self.bump();
            loop {
                implements.push(self.parse_type()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        if matches!(self.peek_kind(), TokenKind::Ident(name) if name == "permits") {
            self.bump();
            loop {
                let _ = self.parse_type()?;
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }

        let mut members: Vec<Stmt> = Vec::new();
        for (ty, cname, cspan) in &components {
            members.push(Stmt::new(
                StmtKind::VarDecl(VarDecl {
                    modifiers: Modifiers {
                        is_private: true,
                        is_final: true,
                        ..Default::default()
                    },
                    ty: Some(ty.clone()),
                    vars: vec![VarDeclarator {
                        name: cname.clone(),
                        dims: 0,
                        init: None,
                        span: *cspan,
                    }],
                    span: *cspan,
                }),
                *cspan,
            ));
        }

        let params: Vec<Param> = components
            .iter()
            .map(|(ty, cname, cspan)| Param {
                ty: Some(ty.clone()),
                name: cname.clone(),
                varargs: false,
                span: *cspan,
            })
            .collect();
        let mut body_stmts = Vec::new();
        for (_, cname, cspan) in &components {
            let this_field = Expr::new(
                ExprKind::Field(Box::new(Expr::new(ExprKind::This, *cspan)), cname.clone()),
                *cspan,
            );
            let arg = Expr::new(ExprKind::Name(vec![cname.clone()]), *cspan);
            let assign = Expr::new(
                ExprKind::Assign {
                    op: AssignOp::Assign,
                    lhs: Box::new(this_field),
                    rhs: Box::new(arg),
                },
                *cspan,
            );
            body_stmts.push(Stmt::new(StmtKind::Expr(assign), *cspan));
        }
        members.push(Stmt::new(
            StmtKind::Method(MethodDecl {
                modifiers: Modifiers {
                    is_public: true,
                    ..Default::default()
                },
                return_type: None,
                name: name.clone(),
                params,
                throws: Vec::new(),
                body: Some(Box::new(Stmt::new(StmtKind::Block(body_stmts), name_span))),
                is_ctor: true,
                span: name_span,
            }),
            name_span,
        ));

        for (ty, cname, cspan) in &components {
            let ret = Expr::new(
                ExprKind::Field(Box::new(Expr::new(ExprKind::This, *cspan)), cname.clone()),
                *cspan,
            );
            members.push(Stmt::new(
                StmtKind::Method(MethodDecl {
                    modifiers: Modifiers {
                        is_public: true,
                        ..Default::default()
                    },
                    return_type: Some(ty.clone()),
                    name: cname.clone(),
                    params: Vec::new(),
                    throws: Vec::new(),
                    body: Some(Box::new(Stmt::new(StmtKind::Return(Some(ret)), *cspan))),
                    is_ctor: false,
                    span: *cspan,
                }),
                *cspan,
            ));
        }

        let string_ty = TypeRef {
            name: vec!["String".to_string()],
            primitive: None,
            dims: 0,
            span: name_span,
        };
        let int_ty = TypeRef {
            name: vec!["int".to_string()],
            primitive: Some(PrimKind::Int),
            dims: 0,
            span: name_span,
        };
        let mut text_parts = vec![Expr::new(
            ExprKind::Literal(Literal::Str(format!("{}[", name))),
            name_span,
        )];
        for (i, (_, cname, cspan)) in components.iter().enumerate() {
            if i > 0 {
                text_parts.push(Expr::new(
                    ExprKind::Literal(Literal::Str(", ".to_string())),
                    *cspan,
                ));
            }
            text_parts.push(Expr::new(
                ExprKind::Literal(Literal::Str(format!("{}=", cname))),
                *cspan,
            ));
            text_parts.push(Expr::new(
                ExprKind::Field(Box::new(Expr::new(ExprKind::This, *cspan)), cname.clone()),
                *cspan,
            ));
        }
        text_parts.push(Expr::new(
            ExprKind::Literal(Literal::Str("]".to_string())),
            name_span,
        ));
        let mut text_expr = text_parts.remove(0);
        for rhs in text_parts {
            text_expr = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    lhs: Box::new(text_expr),
                    rhs: Box::new(rhs),
                },
                name_span,
            );
        }
        members.push(Stmt::new(
            StmtKind::Method(MethodDecl {
                modifiers: Modifiers {
                    is_public: true,
                    ..Default::default()
                },
                return_type: Some(string_ty),
                name: "toString".to_string(),
                params: Vec::new(),
                throws: Vec::new(),
                body: Some(Box::new(Stmt::new(
                    StmtKind::Return(Some(text_expr)),
                    name_span,
                ))),
                is_ctor: false,
                span: name_span,
            }),
            name_span,
        ));

        let mut hash_args = Vec::new();
        for (_, cname, cspan) in &components {
            hash_args.push(Expr::new(
                ExprKind::Field(Box::new(Expr::new(ExprKind::This, *cspan)), cname.clone()),
                *cspan,
            ));
        }
        let hash_call = Expr::new(
            ExprKind::Call {
                target: Box::new(Expr::new(
                    ExprKind::Name(vec![
                        "java".to_string(),
                        "util".to_string(),
                        "Objects".to_string(),
                        "hash".to_string(),
                    ]),
                    name_span,
                )),
                args: hash_args,
            },
            name_span,
        );
        members.push(Stmt::new(
            StmtKind::Method(MethodDecl {
                modifiers: Modifiers {
                    is_public: true,
                    ..Default::default()
                },
                return_type: Some(int_ty),
                name: "hashCode".to_string(),
                params: Vec::new(),
                throws: Vec::new(),
                body: Some(Box::new(Stmt::new(
                    StmtKind::Return(Some(hash_call)),
                    name_span,
                ))),
                is_ctor: false,
                span: name_span,
            }),
            name_span,
        ));

        let point_ty = TypeRef {
            name: vec![name.clone()],
            primitive: None,
            dims: 0,
            span: name_span,
        };
        let o_name = Expr::new(ExprKind::Name(vec!["o".to_string()]), name_span);
        let not_instance = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Box::new(Expr::new(
                    ExprKind::InstanceOf(Box::new(o_name.clone()), point_ty.clone()),
                    name_span,
                )),
            },
            name_span,
        );
        let if_stmt = Stmt::new(
            StmtKind::If {
                cond: not_instance,
                then_branch: Box::new(Stmt::new(
                    StmtKind::Return(Some(Expr::new(
                        ExprKind::Literal(Literal::Bool(false)),
                        name_span,
                    ))),
                    name_span,
                )),
                else_branch: None,
            },
            name_span,
        );
        let other_decl = Stmt::new(
            StmtKind::VarDecl(VarDecl {
                modifiers: Modifiers::default(),
                ty: None,
                vars: vec![VarDeclarator {
                    name: "other".to_string(),
                    dims: 0,
                    init: Some(o_name),
                    span: name_span,
                }],
                span: name_span,
            }),
            name_span,
        );
        let mut eq_expr: Option<Expr> = None;
        for (_, cname, cspan) in &components {
            let left = Expr::new(
                ExprKind::Field(Box::new(Expr::new(ExprKind::This, *cspan)), cname.clone()),
                *cspan,
            );
            let right = Expr::new(
                ExprKind::Field(
                    Box::new(Expr::new(ExprKind::Name(vec!["other".to_string()]), *cspan)),
                    cname.clone(),
                ),
                *cspan,
            );
            let eq = Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                *cspan,
            );
            eq_expr = Some(match eq_expr {
                Some(prev) => Expr::new(
                    ExprKind::Binary {
                        op: BinOp::And,
                        lhs: Box::new(prev),
                        rhs: Box::new(eq),
                    },
                    *cspan,
                ),
                None => eq,
            });
        }
        let eq_expr =
            eq_expr.unwrap_or_else(|| Expr::new(ExprKind::Literal(Literal::Bool(true)), name_span));
        members.push(Stmt::new(
            StmtKind::Method(MethodDecl {
                modifiers: Modifiers {
                    is_public: true,
                    ..Default::default()
                },
                return_type: Some(TypeRef {
                    name: vec!["boolean".to_string()],
                    primitive: Some(PrimKind::Boolean),
                    dims: 0,
                    span: name_span,
                }),
                name: "equals".to_string(),
                params: vec![Param {
                    ty: None,
                    name: "o".to_string(),
                    varargs: false,
                    span: name_span,
                }],
                throws: Vec::new(),
                body: Some(Box::new(Stmt::new(
                    StmtKind::Block(vec![
                        if_stmt,
                        other_decl,
                        Stmt::new(StmtKind::Return(Some(eq_expr)), name_span),
                    ]),
                    name_span,
                ))),
                is_ctor: false,
                span: name_span,
            }),
            name_span,
        ));

        if self.eat(&TokenKind::LBrace) {
            self.class_depth += 1;
            while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                members.push(self.parse_class_member()?);
            }
            self.expect(&TokenKind::RBrace, "'}'")?;
            self.class_depth -= 1;
        }

        let cd = ClassDecl {
            modifiers,
            kind: ClassKind::Class,
            name,
            extends: None,
            implements,
            enum_constants: Vec::new(),
            members,
            span: self.span_from(start),
        };
        Ok(Stmt::new(StmtKind::Class(cd), self.span_from(start)))
    }

    fn parse_class_member(&mut self) -> PResult<Stmt> {
        let start = self.peek().span;
        self.skip_annotations();
        if self.at(&TokenKind::LBrace) {
            // static initializer block
            let b = self.parse_statement()?;
            return Ok(b);
        }
        let mods = self.parse_modifiers();
        if self.at(&TokenKind::LBrace) {
            return self.parse_statement();
        }
        if matches!(
            self.peek_kind(),
            TokenKind::Kw(Kw::Class) | TokenKind::Kw(Kw::Interface) | TokenKind::Kw(Kw::Enum)
        ) {
            let cd = self.parse_class_decl(mods, start)?;
            return Ok(Stmt::new(StmtKind::Class(cd), self.span_from(start)));
        }
        if self.at(&TokenKind::Lt) {
            if let Some(j) = skip_generics(&self.toks, self.pos) {
                self.pos = j;
            }
        }
        // BeanShell allows loose variables and plain statements in a class body.
        if !self.looks_like_decl() {
            return self.parse_statement();
        }
        // Constructor
        if let TokenKind::Ident(_) = self.peek_kind() {
            if matches!(self.peek_at(1).kind, TokenKind::LParen) {
                let (name, _) = self.expect_ident()?;
                let m = self.parse_method_tail(mods, None, name, start)?;
                return Ok(Stmt::new(StmtKind::Method(m), self.span_from(start)));
            }
        }
        let ty = self.parse_type()?;
        if let TokenKind::Ident(_) = self.peek_kind() {
            if matches!(self.peek_at(1).kind, TokenKind::LParen) {
                let (name, _) = self.expect_ident()?;
                let m = self.parse_method_tail(mods, Some(ty), name, start)?;
                return Ok(Stmt::new(StmtKind::Method(m), self.span_from(start)));
            }
        }
        let (name, name_span) = self.expect_ident()?;
        if self.at(&TokenKind::LParen) || self.at_kw(Kw::Throws) {
            let m = self.parse_method_tail(mods, Some(ty), name, start)?;
            return Ok(Stmt::new(StmtKind::Method(m), self.span_from(start)));
        }
        let mut vars = Vec::new();
        let mut vd = VarDeclarator {
            name,
            dims: 0,
            init: None,
            span: name_span,
        };
        self.parse_declarator_rest(&mut vd)?;
        vars.push(vd);
        while self.eat(&TokenKind::Comma) {
            let (name, span) = self.expect_ident()?;
            let mut vd = VarDeclarator {
                name,
                dims: 0,
                init: None,
                span,
            };
            self.parse_declarator_rest(&mut vd)?;
            vars.push(vd);
        }
        self.end_statement()?;
        Ok(Stmt::new(
            StmtKind::VarDecl(VarDecl {
                modifiers: mods,
                ty: Some(ty),
                vars,
                span: self.span_from(start),
            }),
            self.span_from(start),
        ))
    }

    // ---------------------------------------------------------------- types

    fn parse_type(&mut self) -> PResult<TypeRef> {
        let start = self.peek().span;
        let mut ty = self.parse_base_type()?;
        while self.at(&TokenKind::LBracket) && matches!(self.peek_at(1).kind, TokenKind::RBracket) {
            self.bump();
            self.bump();
            ty.dims += 1;
        }
        ty.span = self.span_from(start);
        Ok(ty)
    }

    fn parse_base_type(&mut self) -> PResult<TypeRef> {
        let start = self.peek().span;
        if let TokenKind::Kw(k) = self.peek_kind().clone() {
            if let Some(p) = PrimKind::from_kw(k) {
                self.bump();
                return Ok(TypeRef {
                    name: Vec::new(),
                    primitive: Some(p),
                    dims: 0,
                    span: self.span_from(start),
                });
            }
        }
        let (mut name, _) = self.expect_ident()?;
        let mut parts = vec![name.clone()];
        name.clear();
        loop {
            // Skip generic arguments attached to the segment.
            if self.at(&TokenKind::Lt) {
                if let Some(j) = skip_generics(&self.toks, self.pos) {
                    self.pos = j;
                    self.last_span = self.toks[self.pos - 1].span;
                } else {
                    break;
                }
            }
            if self.at(&TokenKind::Dot) && matches!(self.peek_at(1).kind, TokenKind::Ident(_)) {
                self.bump();
                let (p, _) = self.expect_ident()?;
                parts.push(p);
            } else {
                break;
            }
        }
        Ok(TypeRef {
            name: parts,
            primitive: None,
            dims: 0,
            span: self.span_from(start),
        })
    }

    // ----------------------------------------------------------- expressions

    fn parse_array_initializer_or_expr(&mut self) -> PResult<Expr> {
        if self.at(&TokenKind::LBrace) {
            self.parse_array_initializer()
        } else {
            self.parse_expr()
        }
    }

    fn parse_array_initializer(&mut self) -> PResult<Expr> {
        let start = self.peek().span;
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut items = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            items.push(self.parse_array_initializer_or_expr()?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, "'}'")?;
        Ok(Expr::new(ExprKind::ArrayInit(items), self.span_from(start)))
    }

    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_assignment()
    }

    /// BeanShell @word operator aliases, e.g. "a @gt b".
    fn peek_alias_binop(&self) -> Option<BinOp> {
        if !self.at(&TokenKind::At) {
            return None;
        }
        let name = match &self.peek_at(1).kind {
            TokenKind::Ident(n) => n.as_str(),
            _ => return None,
        };
        Some(match name {
            "gt" => BinOp::Gt,
            "lt" => BinOp::Lt,
            "gteq" => BinOp::Ge,
            "lteq" => BinOp::Le,
            "and" => BinOp::And,
            "or" => BinOp::Or,
            "mod" => BinOp::Mod,
            "pow" => BinOp::Pow,
            "bitwise_and" => BinOp::BitAnd,
            "bitwise_or" => BinOp::BitOr,
            "bitwise_xor" => BinOp::BitXor,
            "left_shift" => BinOp::Shl,
            "right_shift" => BinOp::Shr,
            "right_unsigned_shift" => BinOp::Ushr,
            _ => return None,
        })
    }

    fn peek_alias_assign(&self) -> Option<AssignOp> {
        if !self.at(&TokenKind::At) {
            return None;
        }
        let name = match &self.peek_at(1).kind {
            TokenKind::Ident(n) => n.as_str(),
            _ => return None,
        };
        Some(match name {
            "and_assign" => AssignOp::BitAnd,
            "or_assign" => AssignOp::BitOr,
            "xor_assign" => AssignOp::BitXor,
            "mod_assign" => AssignOp::Mod,
            "pow_assign" => AssignOp::Pow,
            "left_shift_assign" => AssignOp::Shl,
            "right_shift_assign" => AssignOp::Shr,
            "right_unsigned_shift_assign" => AssignOp::Ushr,
            _ => return None,
        })
    }

    fn parse_assignment(&mut self) -> PResult<Expr> {
        let lhs = self.parse_ternary()?;
        if let Some(op) = self.peek_alias_assign() {
            self.bump();
            self.bump();
            let rhs = self.parse_assignment()?;
            let span = lhs.span.merge(rhs.span);
            return Ok(Expr::new(
                ExprKind::Assign {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            ));
        }
        let op = match self.peek_kind() {
            TokenKind::Assign => Some(AssignOp::Assign),
            TokenKind::PlusAssign => Some(AssignOp::Add),
            TokenKind::MinusAssign => Some(AssignOp::Sub),
            TokenKind::StarAssign => Some(AssignOp::Mul),
            TokenKind::PowerAssign => Some(AssignOp::Pow),
            TokenKind::NullCoalesceAssign => Some(AssignOp::Coalesce),
            TokenKind::SlashAssign => Some(AssignOp::Div),
            TokenKind::PercentAssign => Some(AssignOp::Mod),
            TokenKind::AmpAssign => Some(AssignOp::BitAnd),
            TokenKind::PipeAssign => Some(AssignOp::BitOr),
            TokenKind::CaretAssign => Some(AssignOp::BitXor),
            TokenKind::ShlAssign => Some(AssignOp::Shl),
            TokenKind::ShrAssign => Some(AssignOp::Shr),
            TokenKind::UshrAssign => Some(AssignOp::Ushr),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let rhs = self.parse_assignment()?;
            let span = lhs.span.merge(rhs.span);
            return Ok(Expr::new(
                ExprKind::Assign {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            ));
        }
        Ok(lhs)
    }

    fn parse_ternary(&mut self) -> PResult<Expr> {
        let cond = self.parse_binary(0)?;
        if self.at(&TokenKind::Elvis) {
            self.bump();
            let else_expr = self.parse_ternary()?;
            let span = cond.span.merge(else_expr.span);
            return Ok(Expr::new(
                ExprKind::Elvis {
                    cond: Box::new(cond),
                    else_expr: Box::new(else_expr),
                },
                span,
            ));
        }
        if self.at(&TokenKind::Question) {
            self.bump();
            let then_expr = self.parse_ternary()?;
            self.expect(&TokenKind::Colon, "':'")?;
            let else_expr = self.parse_ternary()?;
            let span = cond.span.merge(else_expr.span);
            return Ok(Expr::new(
                ExprKind::Ternary {
                    cond: Box::new(cond),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                },
                span,
            ));
        }
        Ok(cond)
    }

    fn parse_binary(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            if let Some(op) = self.peek_alias_binop() {
                let prec = binop_prec(op);
                if prec >= min_prec {
                    self.bump();
                    self.bump();
                    let rhs = self.parse_binary(prec + 1)?;
                    let span = lhs.span.merge(rhs.span);
                    lhs = Expr::new(
                        ExprKind::Binary {
                            op,
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                        },
                        span,
                    );
                    continue;
                }
                break;
            }
            let (prec, op) = match self.peek_kind() {
                TokenKind::NullCoalesce => (1, BinOp::Coalesce),
                TokenKind::OrOr => (1, BinOp::Or),
                TokenKind::AndAnd => (2, BinOp::And),
                TokenKind::Pipe => (3, BinOp::BitOr),
                TokenKind::Caret => (4, BinOp::BitXor),
                TokenKind::Amp => (5, BinOp::BitAnd),
                TokenKind::Eq => (6, BinOp::Eq),
                TokenKind::Ne => (6, BinOp::Ne),
                TokenKind::Lt => (7, BinOp::Lt),
                TokenKind::Gt => (7, BinOp::Gt),
                TokenKind::Le => (7, BinOp::Le),
                TokenKind::Ge => (7, BinOp::Ge),
                TokenKind::Kw(Kw::Instanceof) => (7, BinOp::Eq),
                TokenKind::Spaceship => (7, BinOp::Cmp),
                TokenKind::Shl => (8, BinOp::Shl),
                TokenKind::Shr => (8, BinOp::Shr),
                TokenKind::Ushr => (8, BinOp::Ushr),
                TokenKind::Plus => (9, BinOp::Add),
                TokenKind::Minus => (9, BinOp::Sub),
                TokenKind::Star => (10, BinOp::Mul),
                TokenKind::Slash => (10, BinOp::Div),
                TokenKind::Percent => (10, BinOp::Mod),
                TokenKind::Power => (11, BinOp::Pow),
                _ => break,
            };
            if prec < min_prec {
                break;
            }
            if matches!(self.peek_kind(), TokenKind::Kw(Kw::Instanceof)) {
                self.bump();
                let ty = self.parse_type()?;
                let span = lhs.span.merge(ty.span);
                if let TokenKind::Ident(name) = self.peek_kind().clone() {
                    self.bump();
                    lhs = Expr::new(ExprKind::InstanceOfPattern(Box::new(lhs), ty, name), span);
                    continue;
                }
                lhs = Expr::new(ExprKind::InstanceOf(Box::new(lhs), ty), span);
                continue;
            }
            self.bump();
            let rhs = self.parse_binary(prec + 1)?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr::new(
                ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            );
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let start = self.peek().span;
        let op = match self.peek_kind() {
            TokenKind::Plus => Some(UnOp::Plus),
            TokenKind::Minus => Some(UnOp::Neg),
            TokenKind::Not => Some(UnOp::Not),
            TokenKind::Tilde => Some(UnOp::BitNot),
            TokenKind::PlusPlus => Some(UnOp::PreInc),
            TokenKind::MinusMinus => Some(UnOp::PreDec),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let expr = self.parse_unary()?;
            let span = self.span_from(start);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.at(&TokenKind::LParen) {
            if let Some(cast) = self.try_parse_cast()? {
                return Ok(cast);
            }
        }
        self.parse_postfix()
    }

    fn try_parse_cast(&mut self) -> PResult<Option<Expr>> {
        let save = self.pos;
        let save_span = self.last_span;
        let start = self.peek().span;
        if !self.eat(&TokenKind::LParen) {
            return Ok(None);
        }
        let ty = match self.parse_type() {
            Ok(t) => t,
            Err(_) => {
                self.pos = save;
                self.last_span = save_span;
                return Ok(None);
            }
        };
        if !self.eat(&TokenKind::RParen) {
            self.pos = save;
            self.last_span = save_span;
            return Ok(None);
        }
        // A cast must be followed by something that can start a unary expr.
        let mut starts_operand = matches!(
            self.peek_kind(),
            TokenKind::Ident(_)
                | TokenKind::Int { .. }
                | TokenKind::Float { .. }
                | TokenKind::Str(_)
                | TokenKind::CharStr { .. }
                | TokenKind::LParen
                | TokenKind::LBrace
                | TokenKind::Not
                | TokenKind::Tilde
                | TokenKind::Kw(Kw::New)
                | TokenKind::Kw(Kw::This)
                | TokenKind::Kw(Kw::Super)
                | TokenKind::Kw(Kw::True)
                | TokenKind::Kw(Kw::False)
                | TokenKind::Kw(Kw::Null)
        );
        // A cast to a primitive type may be followed by a signed operand.
        if ty.primitive.is_some() && matches!(self.peek_kind(), TokenKind::Plus | TokenKind::Minus)
        {
            starts_operand = true;
        }
        if !starts_operand {
            self.pos = save;
            self.last_span = save_span;
            return Ok(None);
        }
        let expr = self.parse_unary()?;
        let span = self.span_from(start);
        Ok(Some(Expr::new(ExprKind::Cast(Box::new(expr), ty), span)))
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::Dot => {
                    self.bump();
                    match self.peek_kind().clone() {
                        TokenKind::Kw(Kw::Class) => {
                            let span = self.peek().span;
                            self.bump();
                            let full = expr.span.merge(span);
                            expr = Expr::new(
                                ExprKind::Field(Box::new(expr), "class".to_string()),
                                full,
                            );
                        }
                        TokenKind::Ident(name) => {
                            let span = self.peek().span;
                            self.bump();
                            let full = expr.span.merge(span);
                            expr = Expr::new(ExprKind::Field(Box::new(expr), name), full);
                        }
                        TokenKind::Kw(Kw::This) => {
                            // Outer-class access, e.g. Foo.this: not supported.
                            let span = self.peek().span;
                            self.bump();
                            let full = expr.span.merge(span);
                            expr = Expr::new(
                                ExprKind::Field(Box::new(expr), "this".to_string()),
                                full,
                            );
                        }
                        TokenKind::Kw(Kw::Super) => {
                            // Field access through super, e.g. o.super.foo
                            let span = self.peek().span;
                            self.bump();
                            let full = expr.span.merge(span);
                            expr = Expr::new(
                                ExprKind::Field(Box::new(expr), "super".to_string()),
                                full,
                            );
                        }
                        TokenKind::Kw(Kw::New) => {
                            // Inner class allocation: outer.new Inner(...)
                            self.bump();
                            let ty = self.parse_base_type()?;
                            let args = self.parse_arguments()?;
                            let full = expr.span.merge(self.last_span);
                            expr = Expr::new(
                                ExprKind::NewInner {
                                    outer: Box::new(expr),
                                    ty,
                                    args,
                                },
                                full,
                            );
                        }
                        TokenKind::Lt => {
                            // Explicit generic method type arguments: obj.<T>m()
                            if let Some(j) = skip_generics(&self.toks, self.pos) {
                                self.pos = j;
                                self.last_span = self.toks[self.pos - 1].span;
                                let (name, span) = self.expect_ident()?;
                                let full = expr.span.merge(span);
                                expr = Expr::new(ExprKind::Field(Box::new(expr), name), full);
                            } else {
                                return Err(self.err("malformed type arguments"));
                            }
                        }
                        other => {
                            return Err(self.err(format!(
                                "Encountered {} but expected a member name.",
                                other.describe()
                            )));
                        }
                    }
                }
                TokenKind::LBracket => {
                    self.bump();
                    if self.at(&TokenKind::ColonColon) {
                        // arr[::step]
                        self.bump();
                        let step = if self.at(&TokenKind::RBracket) {
                            None
                        } else {
                            Some(self.parse_expr()?)
                        };
                        self.expect(&TokenKind::RBracket, "']'")?;
                        let span = expr.span.merge(self.last_span);
                        expr = Expr::new(
                            ExprKind::Slice {
                                base: Box::new(expr),
                                parts: vec![None, None, step],
                            },
                            span,
                        );
                        continue;
                    }
                    if self.at(&TokenKind::Colon) {
                        let mut parts = vec![None];
                        while self.eat(&TokenKind::Colon) {
                            if self.at(&TokenKind::Colon) || self.at(&TokenKind::RBracket) {
                                parts.push(None);
                            } else {
                                parts.push(Some(self.parse_expr()?));
                            }
                        }
                        self.expect(&TokenKind::RBracket, "']'")?;
                        let span = expr.span.merge(self.last_span);
                        expr = Expr::new(
                            ExprKind::Slice {
                                base: Box::new(expr),
                                parts,
                            },
                            span,
                        );
                        continue;
                    }
                    let first = self.parse_expr()?;
                    if self.at(&TokenKind::ColonColon) {
                        self.bump();
                        let step = if self.at(&TokenKind::RBracket) {
                            None
                        } else {
                            Some(self.parse_expr()?)
                        };
                        self.expect(&TokenKind::RBracket, "']'")?;
                        let span = expr.span.merge(self.last_span);
                        expr = Expr::new(
                            ExprKind::Slice {
                                base: Box::new(expr),
                                parts: vec![Some(first), None, step],
                            },
                            span,
                        );
                        continue;
                    }
                    if self.at(&TokenKind::Colon) {
                        let mut parts = vec![Some(first)];
                        while self.eat(&TokenKind::Colon) {
                            if self.at(&TokenKind::Colon) || self.at(&TokenKind::RBracket) {
                                parts.push(None);
                            } else {
                                parts.push(Some(self.parse_expr()?));
                            }
                        }
                        self.expect(&TokenKind::RBracket, "']'")?;
                        let span = expr.span.merge(self.last_span);
                        expr = Expr::new(
                            ExprKind::Slice {
                                base: Box::new(expr),
                                parts,
                            },
                            span,
                        );
                    } else {
                        self.expect(&TokenKind::RBracket, "']'")?;
                        let span = expr.span.merge(self.last_span);
                        expr = Expr::new(ExprKind::Index(Box::new(expr), Box::new(first)), span);
                    }
                }
                TokenKind::ColonColon
                    if matches!(
                        self.peek_at(1).kind,
                        TokenKind::Ident(_) | TokenKind::Kw(Kw::New)
                    ) =>
                {
                    self.bump();
                    let name = match self.peek_kind().clone() {
                        TokenKind::Ident(n) => {
                            self.bump();
                            n
                        }
                        TokenKind::Kw(Kw::New) => {
                            self.bump();
                            "new".to_string()
                        }
                        other => {
                            return Err(self.err(format!(
                                "Encountered {} but expected a method name after '::'.",
                                other.describe()
                            )));
                        }
                    };
                    let span = expr.span.merge(self.last_span);
                    expr = Expr::new(ExprKind::MethodRef(Box::new(expr), name), span);
                }
                TokenKind::QuestionDot => {
                    // Null safe navigation: base?.name
                    self.bump();
                    match self.peek_kind().clone() {
                        TokenKind::Ident(name) => {
                            let span = self.peek().span;
                            self.bump();
                            let full = expr.span.merge(span);
                            expr = Expr::new(ExprKind::SafeField(Box::new(expr), name), full);
                        }
                        other => {
                            return Err(self.err(format!(
                                "Encountered {} but expected a member name after '?.'.",
                                other.describe()
                            )));
                        }
                    }
                }
                TokenKind::LBrace => {
                    // BeanShell map/property access: obj{"key"}
                    self.bump();
                    let idx = self.parse_expr()?;
                    self.expect(&TokenKind::RBrace, "'}'")?;
                    let span = expr.span.merge(self.last_span);
                    expr = Expr::new(ExprKind::Index(Box::new(expr), Box::new(idx)), span);
                }
                TokenKind::LParen => {
                    let args = self.parse_arguments()?;
                    let span = expr.span.merge(self.last_span);
                    expr = Expr::new(
                        ExprKind::Call {
                            target: Box::new(expr),
                            args,
                        },
                        span,
                    );
                }
                TokenKind::PlusPlus => {
                    self.bump();
                    let span = expr.span.merge(self.last_span);
                    expr = Expr::new(
                        ExprKind::Unary {
                            op: UnOp::PostInc,
                            expr: Box::new(expr),
                        },
                        span,
                    );
                }
                TokenKind::MinusMinus => {
                    self.bump();
                    let span = expr.span.merge(self.last_span);
                    expr = Expr::new(
                        ExprKind::Unary {
                            op: UnOp::PostDec,
                            expr: Box::new(expr),
                        },
                        span,
                    );
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_arguments(&mut self) -> PResult<Vec<Expr>> {
        self.expect(&TokenKind::LParen, "'('")?;
        let mut args = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                args.push(self.parse_expr()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen, "')'")?;
        Ok(args)
    }

    /// Recognise "Some.Type[][].class" and primitive class literals.
    fn try_parse_class_literal(&mut self) -> PResult<Option<Expr>> {
        let save = self.pos;
        let save_span = self.last_span;
        if let Ok(ty) = self.parse_type() {
            if self.at(&TokenKind::Dot) && matches!(self.peek_at(1).kind, TokenKind::Kw(Kw::Class))
            {
                self.bump();
                self.bump();
                let span = self.span_from(ty.span);
                let name = Expr::new(ExprKind::Name(vec![ty.display_name()]), ty.span);
                return Ok(Some(Expr::new(
                    ExprKind::Field(Box::new(name), "class".to_string()),
                    span,
                )));
            }
        }
        self.pos = save;
        self.last_span = save_span;
        Ok(None)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let start = self.peek().span;
        match self.peek_kind().clone() {
            TokenKind::Int {
                text,
                radix,
                suffix,
            } => {
                self.bump();
                let lit = parse_int_literal(&text, radix, suffix)
                    .ok_or_else(|| self.err("invalid integer literal"))?;
                Ok(Expr::new(ExprKind::Literal(lit), start))
            }
            TokenKind::Float { text, kind, big } => {
                self.bump();
                let lit = if big {
                    Literal::BigDec(text)
                } else {
                    match kind {
                        FloatKind::Float => match text.parse::<f32>() {
                            Ok(v) => Literal::Float(v),
                            Err(_) => return Err(self.err("invalid float literal")),
                        },
                        FloatKind::Double => match text.parse::<f64>() {
                            Ok(v) => Literal::Double(v),
                            Err(_) => return Err(self.err("invalid double literal")),
                        },
                    }
                };
                Ok(Expr::new(ExprKind::Literal(lit), start))
            }
            TokenKind::Str(s) => {
                self.bump();
                self.parse_string_template(s, start)
            }
            TokenKind::CharStr { raw, decoded } => {
                self.bump();
                let len = raw.chars().count();
                let lit = if len == 1 || (raw.starts_with('\\') && decoded.chars().count() == 1) {
                    match decoded.chars().next() {
                        Some(c) => Literal::Char(c),
                        None => Literal::Str(String::new()),
                    }
                } else {
                    Literal::Str(decoded)
                };
                Ok(Expr::new(ExprKind::Literal(lit), start))
            }
            TokenKind::Kw(Kw::True) => {
                self.bump();
                Ok(Expr::new(ExprKind::Literal(Literal::Bool(true)), start))
            }
            TokenKind::Kw(Kw::False) => {
                self.bump();
                Ok(Expr::new(ExprKind::Literal(Literal::Bool(false)), start))
            }
            TokenKind::Kw(Kw::Null) => {
                self.bump();
                Ok(Expr::new(ExprKind::Literal(Literal::Null), start))
            }
            TokenKind::Kw(k) if PrimKind::from_kw(k).is_some() => {
                if let Some(e) = self.try_parse_class_literal()? {
                    return Ok(e);
                }
                if k == Kw::Void {
                    self.bump();
                    Ok(Expr::new(ExprKind::Literal(Literal::Void), start))
                } else {
                    Err(self.err(format!(
                        "Encountered '{}' but expected an expression.",
                        k.text()
                    )))
                }
            }
            TokenKind::Kw(Kw::This) => {
                self.bump();
                Ok(Expr::new(ExprKind::This, start))
            }
            TokenKind::Kw(Kw::Super) => {
                self.bump();
                Ok(Expr::new(ExprKind::Super, start))
            }
            TokenKind::Kw(Kw::Switch) => self.parse_switch_expr(),
            TokenKind::Kw(Kw::New) => self.parse_new(),
            TokenKind::LParen => {
                if let Some(lam) = self.try_parse_lambda()? {
                    return Ok(lam);
                }
                self.bump();
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "')'")?;
                Ok(e)
            }
            TokenKind::LBrace => self.parse_array_initializer(),
            TokenKind::Ident(_) => {
                if let Some(e) = self.try_parse_class_literal()? {
                    return Ok(e);
                }
                if matches!(self.peek_at(1).kind, TokenKind::Arrow) {
                    if let Some(lam) = self.try_parse_lambda()? {
                        return Ok(lam);
                    }
                }
                let mut parts = Vec::new();
                loop {
                    let (name, _) = self.expect_ident()?;
                    parts.push(name);
                    if self.at(&TokenKind::Dot)
                        && matches!(self.peek_at(1).kind, TokenKind::Ident(_))
                    {
                        self.bump();
                    } else {
                        break;
                    }
                }
                Ok(Expr::new(ExprKind::Name(parts), self.span_from(start)))
            }
            other => Err(self.err(format!(
                "Encountered {} but expected an expression.",
                other.describe()
            ))),
        }
    }

    fn try_parse_lambda(&mut self) -> PResult<Option<Expr>> {
        let start = self.peek().span;
        // Single identifier parameter.
        if let TokenKind::Ident(name) = self.peek_kind().clone() {
            if matches!(self.peek_at(1).kind, TokenKind::Arrow) {
                self.bump();
                self.bump();
                let body = self.parse_lambda_body()?;
                let span = self.span_from(start);
                return Ok(Some(Expr::new(
                    ExprKind::Lambda {
                        params: vec![Param {
                            ty: None,
                            name,
                            varargs: false,
                            span: start,
                        }],
                        body: Box::new(body),
                    },
                    span,
                )));
            }
            return Ok(None);
        }
        if !self.at(&TokenKind::LParen) {
            return Ok(None);
        }
        let j = match matching_paren(&self.toks, self.pos) {
            Some(j) => j,
            None => return Ok(None),
        };
        if !matches!(self.toks[j + 1].kind, TokenKind::Arrow) {
            return Ok(None);
        }
        self.bump(); // '('
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                // Optional type before the name.
                let mut ty = None;
                if self.looks_like_type_start()
                    && matches!(self.peek_at(1).kind, TokenKind::Ident(_))
                {
                    let parsed = self.parse_type()?;
                    // Java 11: (var x) -> ... means inferred type.
                    if !(parsed.dims == 0 && parsed.base_name() == "var") {
                        ty = Some(parsed);
                    }
                }
                let (name, span) = self.expect_ident()?;
                params.push(Param {
                    ty,
                    name,
                    varargs: false,
                    span,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen, "')'")?;
        self.expect(&TokenKind::Arrow, "'->'")?;
        let body = self.parse_lambda_body()?;
        let span = self.span_from(start);
        Ok(Some(Expr::new(
            ExprKind::Lambda {
                params,
                body: Box::new(body),
            },
            span,
        )))
    }

    fn parse_lambda_body(&mut self) -> PResult<Stmt> {
        if self.at(&TokenKind::LBrace) {
            self.parse_statement()
        } else {
            let start = self.peek().span;
            let e = self.parse_expr()?;
            Ok(Stmt::new(StmtKind::Return(Some(e)), self.span_from(start)))
        }
    }

    fn parse_new(&mut self) -> PResult<Expr> {
        let start = self.peek().span;
        self.bump(); // 'new'
        if self.at(&TokenKind::LBrace) {
            // Untyped collection literal: new {} / new {"k" = 1}
            let init = match self.parse_array_initializer()?.kind {
                ExprKind::ArrayInit(items) => items,
                _ => Vec::new(),
            };
            let ty = TypeRef {
                name: vec!["Object".to_string()],
                primitive: None,
                dims: 1,
                span: start,
            };
            let span = self.span_from(start);
            return Ok(Expr::new(
                ExprKind::NewArray {
                    ty,
                    dims: vec![None],
                    init: Some(init),
                },
                span,
            ));
        }
        let base = self.parse_base_type()?;
        let mut dims: Vec<Option<Expr>> = Vec::new();
        while self.at(&TokenKind::LBracket) {
            self.bump();
            if self.at(&TokenKind::RBracket) {
                self.bump();
                dims.push(None);
            } else {
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RBracket, "']'")?;
                dims.push(Some(e));
            }
        }
        if dims.is_empty() && self.at(&TokenKind::LBrace) {
            // BeanShell: "new int {}" or "new List {}"
            let mut ty = base;
            ty.dims = 1;
            let init = match self.parse_array_initializer()?.kind {
                ExprKind::ArrayInit(items) => Some(items),
                _ => None,
            };
            let span = self.span_from(start);
            return Ok(Expr::new(
                ExprKind::NewArray {
                    ty,
                    dims: vec![None],
                    init,
                },
                span,
            ));
        }
        if !dims.is_empty() {
            let mut ty = base;
            ty.dims = dims.len();
            let init = if self.at(&TokenKind::LBrace) {
                let arr = self.parse_array_initializer()?;
                if let ExprKind::ArrayInit(items) = arr.kind {
                    Some(items)
                } else {
                    None
                }
            } else {
                None
            };
            let span = self.span_from(start);
            return Ok(Expr::new(ExprKind::NewArray { ty, dims, init }, span));
        }
        if !self.at(&TokenKind::LParen) {
            return Err(self.err("Encountered {} but expected '(' after 'new' type."));
        }
        let args = self.parse_arguments()?;
        let anon = if self.at(&TokenKind::LBrace) {
            let cstart = self.peek().span;
            let mut cd = ClassDecl {
                modifiers: Modifiers::default(),
                kind: ClassKind::Class,
                name: format!("$anon{}", cstart.line),
                extends: Some(base.clone()),
                implements: Vec::new(),
                enum_constants: Vec::new(),
                members: Vec::new(),
                span: cstart,
            };
            self.bump();
            while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                cd.members.push(self.parse_class_member()?);
            }
            self.expect(&TokenKind::RBrace, "'}'")?;
            Some(Box::new(cd))
        } else {
            None
        };
        let span = self.span_from(start);
        Ok(Expr::new(
            ExprKind::New {
                ty: base,
                args,
                anon,
            },
            span,
        ))
    }
}

/// If a type starts at index i, return the index just past it.
fn scan_type_end(toks: &[Token], i: usize) -> Option<usize> {
    match &toks[i].kind {
        TokenKind::Kw(k) => {
            if PrimKind::from_kw(*k).is_none() {
                return None;
            }
            let mut j = i + 1;
            while matches!(toks[j].kind, TokenKind::LBracket)
                && matches!(toks[j + 1].kind, TokenKind::RBracket)
            {
                j += 2;
            }
            Some(j)
        }
        TokenKind::Ident(_) => {
            let mut j = i + 1;
            loop {
                match &toks[j].kind {
                    TokenKind::Dot if matches!(toks[j + 1].kind, TokenKind::Ident(_)) => j += 2,
                    TokenKind::Lt => match skip_generics(toks, j) {
                        Some(k) => j = k,
                        None => return None,
                    },
                    TokenKind::LBracket if matches!(toks[j + 1].kind, TokenKind::RBracket) => {
                        j += 2
                    }
                    _ => break,
                }
            }
            Some(j)
        }
        _ => None,
    }
}

fn binop_prec(op: BinOp) -> u8 {
    match op {
        BinOp::Coalesce | BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::BitOr => 3,
        BinOp::BitXor => 4,
        BinOp::BitAnd => 5,
        BinOp::Eq | BinOp::Ne => 6,
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Cmp => 7,
        BinOp::Shl | BinOp::Shr | BinOp::Ushr => 8,
        BinOp::Add | BinOp::Sub => 9,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 10,
        BinOp::Pow => 11,
    }
}

fn parse_int_literal(text: &str, radix: u32, suffix: Option<char>) -> Option<Literal> {
    let parsed = i128::from_str_radix(text, radix).ok();
    match suffix.map(|c| c.to_ascii_lowercase()) {
        Some('l') => Some(Literal::Long(parsed.unwrap_or(0) as i64)),
        Some('i') => Some(Literal::Int(parsed.unwrap_or(0) as i32 as i64)),
        Some('s') => Some(Literal::Short(parsed.unwrap_or(0) as i16)),
        Some('o') => Some(Literal::Byte(parsed.unwrap_or(0) as i8)),
        Some('w') => Some(Literal::BigInt(text.to_string())),
        _ => match parsed {
            Some(v) if v >= i32::MIN as i128 && v <= i32::MAX as i128 => {
                Some(Literal::Int(v as i32 as i64))
            }
            Some(v) if v >= i64::MIN as i128 && v <= i64::MAX as i128 => {
                Some(Literal::Long(v as i64))
            }
            _ => Some(Literal::BigInt(text.to_string())),
        },
    }
}

/// If a '<' starts a generic argument list at index i, return the index just
/// past its matching '>'.
fn skip_generics(toks: &[Token], i: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut j = i;
    while j < toks.len() {
        match toks[j].kind {
            TokenKind::Lt => {
                depth += 1;
                j += 1;
            }
            TokenKind::Gt => {
                depth -= 1;
                j += 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            TokenKind::Shr => {
                depth -= 2;
                j += 1;
                if depth <= 0 {
                    return Some(j);
                }
            }
            TokenKind::Ushr => {
                depth -= 3;
                j += 1;
                if depth <= 0 {
                    return Some(j);
                }
            }
            TokenKind::Eof => return None,
            _ => {
                j += 1;
            }
        }
    }
    None
}

/// Index of the ')' matching the '(' at index i.
fn matching_paren(toks: &[Token], i: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut j = i;
    while j < toks.len() {
        match toks[j].kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            TokenKind::Eof => return None,
            _ => {}
        }
        j += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) {
        match parse(src) {
            Ok(_) => {}
            Err(e) => panic!("parse failed for {:?}: {}", src, e),
        }
    }

    #[test]
    fn parses_basics() {
        ok("a = 1;");
        ok("int a = 1;");
        ok("String s = \"x\" + 1;");
        ok("if (a) { b(); } else c();");
        ok("for (int i = 0; i < 10; i++) { print(i); }");
        ok("for (int i : new int[] {1,2,3}) print(i);");
        ok("while (true) break;");
        ok("switch (x) { case 1: a(); break; default: b(); }");
        ok("foo() { return 1; }");
        ok("int foo(int a, String... rest) { return a; }");
        ok("class A extends B implements C { int x; A() { super(); } void f() {} }");
        ok("try { a(); } catch (Exception e) { b(); } finally { c(); }");
        ok("Object o = new int[10];");
        ok("Object o = new int[] {1,2};");
        ok("Object o = new Foo(1, 2);");
        ok("Object o = new Foo() { void f() {} };");
        ok("a.b.c = 1;");
        ok("x = (int) y;");
        ok("x = y instanceof String;");
        ok("x = a ? b : c;");
        ok("x++; --y;");
        ok("Runnable r = () -> { };");
        ok("Runnable r = (a, b) -> a;");
        ok("import java.util.*; import static java.lang.Math.*;");
        ok("synchronized (this) { }");
        ok("do { a(); } while (b);");
        ok("outer: for (;;) { break outer; }");
        ok("x = new int[2][3];");
        ok("x = {1, 2, 3};");
        ok("this.foo(1);");
    }
}

/// Find the matching '}' for a template expression, skipping strings,
/// chars, text blocks, and comments.
fn find_template_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    let mut depth = 0usize;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                if i + 2 < chars.len() && chars[i + 1] == '"' && chars[i + 2] == '"' {
                    i += 3;
                    while i + 2 < chars.len() {
                        if chars[i] == '"' && chars[i + 1] == '"' && chars[i + 2] == '"' {
                            i += 3;
                            break;
                        }
                        i += 1;
                    }
                } else {
                    i += 1;
                    while i < chars.len() {
                        match chars[i] {
                            '\\' => i += 2,
                            '"' => {
                                i += 1;
                                break;
                            }
                            _ => i += 1,
                        }
                    }
                }
            }
            '\'' => {
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        '\\' => i += 2,
                        '\'' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => {
                i += 2;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                i += 2;
                while i + 1 < chars.len() {
                    if chars[i] == '*' && chars[i + 1] == '/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            '{' => {
                depth += 1;
                i += 1;
            }
            '}' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}
