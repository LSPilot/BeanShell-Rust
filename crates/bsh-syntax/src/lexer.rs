//! Hand written lexer for the BeanShell (Java-like) grammar.

use crate::error::SyntaxError;
use crate::span::Span;
use crate::token::{FloatKind, Kw, Token, TokenKind};

pub fn tokenize(src: &str) -> Result<Vec<Token>, SyntaxError> {
    Lexer::new(src).run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
}

impl Lexer {
    fn new(src: &str) -> Self {
        Lexer {
            chars: src.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.chars.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn span(&self, start: usize, line: u32, col: u32) -> Span {
        Span::new(start as u32, self.pos as u32, line, col)
    }

    fn err(&self, msg: impl Into<String>, start: usize, line: u32, col: u32) -> SyntaxError {
        SyntaxError::new(msg, self.span(start, line, col))
    }

    fn run(mut self) -> Result<Vec<Token>, SyntaxError> {
        let mut out = Vec::new();
        loop {
            self.skip_trivia()?;
            let start = self.pos;
            let line = self.line;
            let col = self.col;
            let c = match self.peek() {
                Some(c) => c,
                None => {
                    out.push(Token::new(TokenKind::Eof, self.span(start, line, col)));
                    return Ok(out);
                }
            };
            let kind = if is_ident_start(c) {
                let s = self.lex_ident();
                match Kw::from_str(&s) {
                    Some(k) => TokenKind::Kw(k),
                    None => TokenKind::Ident(s),
                }
            } else if c.is_ascii_digit()
                || (c == '.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()))
            {
                self.lex_number(start, line, col)?
            } else if c == '"' {
                self.lex_string(start, line, col)?
            } else if c == '\'' {
                self.lex_char(start, line, col)?
            } else {
                self.lex_operator(start, line, col)?
            };
            out.push(Token::new(kind, self.span(start, line, col)));
        }
    }

    fn skip_trivia(&mut self) -> Result<(), SyntaxError> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some('#') if self.peek_at(1) == Some('!') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    let start = self.pos;
                    let line = self.line;
                    let col = self.col;
                    self.bump();
                    self.bump();
                    let mut closed = false;
                    while let Some(c) = self.bump() {
                        if c == '*' && self.peek() == Some('/') {
                            self.bump();
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        return Err(self.err("unterminated comment", start, line, col));
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn lex_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if is_ident_part(c) {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        s
    }

    fn lex_number(&mut self, start: usize, line: u32, col: u32) -> Result<TokenKind, SyntaxError> {
        let mut radix = 10u32;
        let mut text = String::new();

        if self.peek() == Some('0') {
            match self.peek_at(1) {
                Some('x') | Some('X') => {
                    self.bump();
                    self.bump();
                    radix = 16;
                    let mut digits = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_hexdigit() || c == '_' {
                            if c != '_' {
                                digits.push(c);
                            }
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    if digits.is_empty() {
                        return Err(self.err("malformed hexadecimal literal", start, line, col));
                    }
                    let suffix = self.eat_int_suffix();
                    return Ok(TokenKind::Int {
                        text: digits,
                        radix,
                        suffix,
                    });
                }
                Some('b') | Some('B') => {
                    self.bump();
                    self.bump();
                    radix = 2;
                    let mut digits = String::new();
                    while let Some(c) = self.peek() {
                        if c == '0' || c == '1' || c == '_' {
                            if c != '_' {
                                digits.push(c);
                            }
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    if digits.is_empty() {
                        return Err(self.err("malformed binary literal", start, line, col));
                    }
                    let suffix = self.eat_int_suffix();
                    return Ok(TokenKind::Int {
                        text: digits,
                        radix,
                        suffix,
                    });
                }
                Some(d) if d.is_ascii_digit() => {
                    // Octal is the Java rule for a leading zero; be lenient and
                    // fall back to decimal when an 8/9 shows up.
                    let mut digits = String::new();
                    digits.push('0');
                    self.bump();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_digit() || c == '_' {
                            if c != '_' {
                                digits.push(c);
                            }
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    if digits.chars().all(|c| ('0'..='7').contains(&c)) {
                        radix = 8;
                        let trimmed = digits.trim_start_matches('0').to_string();
                        text = if trimmed.is_empty() {
                            "0".to_string()
                        } else {
                            trimmed
                        };
                    } else {
                        text = digits;
                    }
                    let suffix = self.eat_int_suffix();
                    return Ok(TokenKind::Int {
                        text,
                        radix,
                        suffix,
                    });
                }
                _ => {}
            }
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                if c != '_' {
                    text.push(c);
                }
                self.bump();
            } else {
                break;
            }
        }
        let mut is_float = false;
        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) {
            is_float = true;
            text.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() || c == '_' {
                    if c != '_' {
                        text.push(c);
                    }
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let save = self.pos;
            let save_line = self.line;
            let save_col = self.col;
            let mut exp = String::from("e");
            self.bump();
            if matches!(self.peek(), Some('+') | Some('-')) {
                exp.push(self.bump().unwrap());
            }
            let mut any = false;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    exp.push(c);
                    any = true;
                    self.bump();
                } else {
                    break;
                }
            }
            if any {
                is_float = true;
                text.push_str(&exp);
            } else {
                self.pos = save;
                self.line = save_line;
                self.col = save_col;
            }
        }
        let mut kind = FloatKind::Double;
        let mut big = false;
        let mut suffix: Option<char> = None;
        match self.peek() {
            Some(c @ ('l' | 'L' | 'o' | 'O' | 's' | 'S' | 'i' | 'I' | 'w' | 'W')) => {
                suffix = Some(c);
                self.bump();
            }
            Some('f') | Some('F') => {
                is_float = true;
                kind = FloatKind::Float;
                self.bump();
            }
            Some('d') | Some('D') => {
                is_float = true;
                self.bump();
            }
            _ => {}
        }
        if suffix == Some('w') || suffix == Some('W') {
            big = true;
        }
        if is_float || big {
            Ok(TokenKind::Float { text, kind, big })
        } else {
            Ok(TokenKind::Int {
                text,
                radix,
                suffix,
            })
        }
    }

    fn lex_escape(
        &mut self,
        out: &mut String,
        start: usize,
        line: u32,
        col: u32,
    ) -> Result<(), SyntaxError> {
        match self.bump() {
            None => Err(self.err("unterminated escape sequence", start, line, col)),
            Some('n') => {
                out.push('\n');
                Ok(())
            }
            Some('t') => {
                out.push('\t');
                Ok(())
            }
            Some('r') => {
                out.push('\r');
                Ok(())
            }
            Some('b') => {
                out.push('\u{8}');
                Ok(())
            }
            Some('f') => {
                out.push('\u{c}');
                Ok(())
            }
            Some('\\') => {
                out.push('\\');
                Ok(())
            }
            Some('\'') => {
                out.push('\'');
                Ok(())
            }
            Some('"') => {
                out.push('"');
                Ok(())
            }
            Some('$') => {
                // Kotlin-style escaped dollar; parser turns the sentinel into '$'.
                out.push('\u{E000}');
                Ok(())
            }
            Some('u') => {
                let mut hex = String::new();
                for _ in 0..4 {
                    match self.peek() {
                        Some(c) if c.is_ascii_hexdigit() => {
                            hex.push(c);
                            self.bump();
                        }
                        _ => return Err(self.err("malformed unicode escape", start, line, col)),
                    }
                }
                let v = u32::from_str_radix(&hex, 16)
                    .map_err(|_| self.err("malformed unicode escape", start, line, col))?;
                match char::from_u32(v) {
                    Some(c) => {
                        out.push(c);
                        Ok(())
                    }
                    None => Err(self.err("invalid unicode escape", start, line, col)),
                }
            }
            Some(c @ '0'..='7') => {
                let mut v = c.to_digit(8).unwrap();
                for _ in 0..2 {
                    match self.peek() {
                        Some(d @ '0'..='7') => {
                            v = v * 8 + d.to_digit(8).unwrap();
                            self.bump();
                        }
                        _ => break,
                    }
                }
                match char::from_u32(v) {
                    Some(c) => {
                        out.push(c);
                        Ok(())
                    }
                    None => Err(self.err("invalid octal escape", start, line, col)),
                }
            }
            Some(other) => {
                // BeanShell keeps unknown escapes as the escaped character.
                out.push(other);
                Ok(())
            }
        }
    }

    fn lex_string(&mut self, start: usize, line: u32, col: u32) -> Result<TokenKind, SyntaxError> {
        self.bump(); // opening quote
                     // Java text block: """ ... """
        if self.peek() == Some('"') && self.peek_at(1) == Some('"') {
            self.bump();
            self.bump();
            let mut s = String::new();
            loop {
                if self.peek() == Some('"')
                    && self.peek_at(1) == Some('"')
                    && self.peek_at(2) == Some('"')
                {
                    self.bump();
                    self.bump();
                    self.bump();
                    let body = s.strip_prefix('\n').unwrap_or(&s).to_string();
                    return Ok(TokenKind::Str(body));
                }
                match self.bump() {
                    Some(c) => s.push(c),
                    None => return Err(self.err("unterminated text block", start, line, col)),
                }
            }
        }
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string literal", start, line, col)),
                Some('"') => {
                    self.bump();
                    return Ok(TokenKind::Str(s));
                }
                Some('\\') => {
                    self.bump();
                    self.lex_escape(&mut s, start, line, col)?;
                }
                Some('\n') => {
                    return Err(self.err("unterminated string literal", start, line, col))
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    s.push('$');
                    self.bump();
                    s.push('{');
                    self.bump();
                    self.lex_template_body(&mut s, start, line, col)?;
                }
                Some(c) => {
                    s.push(c);
                    self.bump();
                }
            }
        }
    }

    /// Copy a template expression body; braces inside strings/comments are ignored.
    fn lex_template_body(
        &mut self,
        out: &mut String,
        start: usize,
        line: u32,
        col: u32,
    ) -> Result<(), SyntaxError> {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string template", start, line, col)),
                Some('"') => {
                    if self.peek_at(1) == Some('"') && self.peek_at(2) == Some('"') {
                        for _ in 0..3 {
                            out.push('"');
                            self.bump();
                        }
                        loop {
                            if self.peek() == Some('"')
                                && self.peek_at(1) == Some('"')
                                && self.peek_at(2) == Some('"')
                            {
                                for _ in 0..3 {
                                    out.push('"');
                                    self.bump();
                                }
                                break;
                            }
                            match self.bump() {
                                Some(c) => out.push(c),
                                None => {
                                    return Err(self.err(
                                        "unterminated text block",
                                        start,
                                        line,
                                        col,
                                    ))
                                }
                            }
                        }
                    } else {
                        out.push('"');
                        self.bump();
                        loop {
                            match self.peek() {
                                None => {
                                    return Err(self.err(
                                        "unterminated string in template",
                                        start,
                                        line,
                                        col,
                                    ))
                                }
                                Some('"') => {
                                    out.push('"');
                                    self.bump();
                                    break;
                                }
                                Some('\\') => {
                                    out.push('\\');
                                    self.bump();
                                    match self.bump() {
                                        Some(c) => out.push(c),
                                        None => {
                                            return Err(self.err(
                                                "unterminated string in template",
                                                start,
                                                line,
                                                col,
                                            ))
                                        }
                                    }
                                }
                                Some(c) => {
                                    out.push(c);
                                    self.bump();
                                }
                            }
                        }
                    }
                }
                Some('\'') => {
                    out.push('\'');
                    self.bump();
                    loop {
                        match self.peek() {
                            None => {
                                return Err(self.err(
                                    "unterminated char in template",
                                    start,
                                    line,
                                    col,
                                ))
                            }
                            Some('\'') => {
                                out.push('\'');
                                self.bump();
                                break;
                            }
                            Some('\\') => {
                                out.push('\\');
                                self.bump();
                                match self.bump() {
                                    Some(c) => out.push(c),
                                    None => {
                                        return Err(self.err(
                                            "unterminated char in template",
                                            start,
                                            line,
                                            col,
                                        ))
                                    }
                                }
                            }
                            Some(c) => {
                                out.push(c);
                                self.bump();
                            }
                        }
                    }
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while let Some(c) = self.peek() {
                        out.push(c);
                        self.bump();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    out.push('/');
                    self.bump();
                    out.push('*');
                    self.bump();
                    loop {
                        match self.peek() {
                            None => {
                                return Err(self.err(
                                    "unterminated comment in template",
                                    start,
                                    line,
                                    col,
                                ))
                            }
                            Some('*') if self.peek_at(1) == Some('/') => {
                                out.push('*');
                                self.bump();
                                out.push('/');
                                self.bump();
                                break;
                            }
                            Some(c) => {
                                out.push(c);
                                self.bump();
                            }
                        }
                    }
                }
                Some('{') => {
                    depth += 1;
                    out.push('{');
                    self.bump();
                }
                Some('}') => {
                    if depth == 0 {
                        out.push('}');
                        self.bump();
                        return Ok(());
                    }
                    depth -= 1;
                    out.push('}');
                    self.bump();
                }
                Some(c) => {
                    out.push(c);
                    self.bump();
                }
            }
        }
    }

    fn lex_char(&mut self, start: usize, line: u32, col: u32) -> Result<TokenKind, SyntaxError> {
        self.bump(); // opening quote
        let mut raw = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated character literal", start, line, col)),
                Some('\'') => {
                    self.bump();
                    break;
                }
                Some('\n') => {
                    return Err(self.err("unterminated character literal", start, line, col))
                }
                Some('\\') => {
                    raw.push('\\');
                    self.bump();
                    match self.bump() {
                        Some(c) => raw.push(c),
                        None => {
                            return Err(self.err(
                                "unterminated character literal",
                                start,
                                line,
                                col,
                            ))
                        }
                    }
                }
                Some(c) => {
                    raw.push(c);
                    self.bump();
                }
            }
        }
        let decoded = decode_literal_text(&raw, start, line, col)?;
        Ok(TokenKind::CharStr { raw, decoded })
    }

    fn eat_int_suffix(&mut self) -> Option<char> {
        match self.peek() {
            Some(c @ ('o' | 'O' | 's' | 'S' | 'i' | 'I' | 'l' | 'L' | 'w' | 'W')) => {
                self.bump();
                Some(c)
            }
            _ => None,
        }
    }

    fn lex_operator(
        &mut self,
        start: usize,
        line: u32,
        col: u32,
    ) -> Result<TokenKind, SyntaxError> {
        use TokenKind::*;
        let c = self.bump().unwrap();
        let n1 = self.peek();
        let n2 = self.peek_at(1);
        let n3 = self.peek_at(2);
        let kind = match c {
            '(' => LParen,
            ')' => RParen,
            '{' => LBrace,
            '}' => RBrace,
            '[' => LBracket,
            ']' => RBracket,
            ';' => Semi,
            ',' => Comma,
            '?' => match n1 {
                Some('.') => {
                    self.bump();
                    QuestionDot
                }
                Some('?') => {
                    self.bump();
                    if self.peek() == Some('=') {
                        self.bump();
                        NullCoalesceAssign
                    } else {
                        NullCoalesce
                    }
                }
                Some(':') => {
                    self.bump();
                    Elvis
                }
                _ => Question,
            },
            '@' => At,
            '~' => Tilde,
            ':' => {
                if n1 == Some(':') {
                    self.bump();
                    ColonColon
                } else {
                    Colon
                }
            }
            '.' => {
                if n1 == Some('.') && n2 == Some('.') {
                    self.bump();
                    self.bump();
                    Ellipsis
                } else {
                    Dot
                }
            }
            '+' => match n1 {
                Some('+') => {
                    self.bump();
                    PlusPlus
                }
                Some('=') => {
                    self.bump();
                    PlusAssign
                }
                _ => Plus,
            },
            '-' => match n1 {
                Some('-') => {
                    self.bump();
                    MinusMinus
                }
                Some('=') => {
                    self.bump();
                    MinusAssign
                }
                Some('>') => {
                    self.bump();
                    Arrow
                }
                _ => Minus,
            },
            '*' => match n1 {
                Some('*') => {
                    self.bump();
                    if self.peek() == Some('=') {
                        self.bump();
                        PowerAssign
                    } else {
                        Power
                    }
                }
                Some('=') => {
                    self.bump();
                    StarAssign
                }
                _ => Star,
            },
            '/' => {
                if n1 == Some('=') {
                    self.bump();
                    SlashAssign
                } else {
                    Slash
                }
            }
            '%' => {
                if n1 == Some('=') {
                    self.bump();
                    PercentAssign
                } else {
                    Percent
                }
            }
            '=' => {
                if n1 == Some('=') {
                    self.bump();
                    Eq
                } else {
                    Assign
                }
            }
            '!' => {
                if n1 == Some('=') {
                    self.bump();
                    Ne
                } else {
                    Not
                }
            }
            '&' => match n1 {
                Some('&') => {
                    self.bump();
                    AndAnd
                }
                Some('=') => {
                    self.bump();
                    AmpAssign
                }
                _ => Amp,
            },
            '|' => match n1 {
                Some('|') => {
                    self.bump();
                    OrOr
                }
                Some('=') => {
                    self.bump();
                    PipeAssign
                }
                _ => Pipe,
            },
            '^' => {
                if n1 == Some('=') {
                    self.bump();
                    CaretAssign
                } else {
                    Caret
                }
            }
            '<' => match (n1, n2) {
                (Some('='), Some('>')) => {
                    self.bump();
                    self.bump();
                    Spaceship
                }
                (Some('<'), Some('=')) => {
                    self.bump();
                    self.bump();
                    ShlAssign
                }
                (Some('<'), _) => {
                    self.bump();
                    Shl
                }
                (Some('='), _) => {
                    self.bump();
                    Le
                }
                _ => Lt,
            },
            '>' => match (n1, n2, n3) {
                (Some('>'), Some('>'), Some('=')) => {
                    self.bump();
                    self.bump();
                    self.bump();
                    UshrAssign
                }
                (Some('>'), Some('>'), _) => {
                    self.bump();
                    self.bump();
                    Ushr
                }
                (Some('>'), Some('='), _) => {
                    self.bump();
                    self.bump();
                    ShrAssign
                }
                (Some('>'), _, _) => {
                    self.bump();
                    Shr
                }
                (Some('='), _, _) => {
                    self.bump();
                    Ge
                }
                _ => Gt,
            },
            other => {
                return Err(self.err(
                    format!("unexpected character '{}'", other),
                    start,
                    line,
                    col,
                ));
            }
        };
        Ok(kind)
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

fn is_ident_part(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric()
}

/// Decode Java escape sequences in a single quoted literal body.
fn decode_literal_text(
    raw: &str,
    start: usize,
    line: u32,
    col: u32,
) -> Result<String, SyntaxError> {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mkerr = |msg: &str| {
        SyntaxError::new(
            msg.to_string(),
            Span::new(start as u32, start as u32, line, col),
        )
    };
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let e = match chars.get(i) {
            Some(e) => *e,
            None => return Err(mkerr("unterminated escape sequence")),
        };
        i += 1;
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            'u' => {
                let mut hex = String::new();
                for _ in 0..4 {
                    match chars.get(i) {
                        Some(c) if c.is_ascii_hexdigit() => {
                            hex.push(*c);
                            i += 1;
                        }
                        _ => return Err(mkerr("malformed unicode escape")),
                    }
                }
                let v =
                    u32::from_str_radix(&hex, 16).map_err(|_| mkerr("malformed unicode escape"))?;
                match char::from_u32(v) {
                    Some(c) => out.push(c),
                    None => return Err(mkerr("invalid unicode escape")),
                }
            }
            '0'..='7' => {
                let mut v = e.to_digit(8).unwrap();
                for _ in 0..2 {
                    match chars.get(i) {
                        Some(d) if ('0'..='7').contains(d) => {
                            v = v * 8 + d.to_digit(8).unwrap();
                            i += 1;
                        }
                        _ => break,
                    }
                }
                match char::from_u32(v) {
                    Some(c) => out.push(c),
                    None => return Err(mkerr("invalid octal escape")),
                }
            }
            other => out.push(other),
        }
    }
    Ok(out)
}
