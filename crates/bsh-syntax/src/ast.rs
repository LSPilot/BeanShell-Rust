//! Abstract syntax tree for the BeanShell language.
//!
//! The tree is deliberately close to the Java/BeanShell grammar: statements
//! and expressions are wrapper structs carrying a source span so that the
//! evaluator can attach line numbers to run time errors.

use crate::span::Span;
use crate::token::Kw;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimKind {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
    Void,
}

impl PrimKind {
    pub fn name(self) -> &'static str {
        match self {
            PrimKind::Boolean => "boolean",
            PrimKind::Byte => "byte",
            PrimKind::Char => "char",
            PrimKind::Short => "short",
            PrimKind::Int => "int",
            PrimKind::Long => "long",
            PrimKind::Float => "float",
            PrimKind::Double => "double",
            PrimKind::Void => "void",
        }
    }

    pub fn from_kw(kw: Kw) -> Option<PrimKind> {
        Some(match kw {
            Kw::Boolean => PrimKind::Boolean,
            Kw::Byte => PrimKind::Byte,
            Kw::Char => PrimKind::Char,
            Kw::Short => PrimKind::Short,
            Kw::Int => PrimKind::Int,
            Kw::Long => PrimKind::Long,
            Kw::Float => PrimKind::Float,
            Kw::Double => PrimKind::Double,
            Kw::Void => PrimKind::Void,
            _ => return None,
        })
    }
}

/// A (possibly array) type reference. Generic arguments are erased.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeRef {
    pub name: Vec<String>,
    pub primitive: Option<PrimKind>,
    pub dims: usize,
    pub span: Span,
}

impl TypeRef {
    pub fn base_name(&self) -> String {
        self.name.join(".")
    }

    pub fn display_name(&self) -> String {
        let mut s = match self.primitive {
            Some(p) => p.name().to_string(),
            None => self.base_name(),
        };
        for _ in 0..self.dims {
            s.push_str("[]");
        }
        s
    }

    pub fn is_void(&self) -> bool {
        self.primitive == Some(PrimKind::Void) && self.dims == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub is_public: bool,
    pub is_private: bool,
    pub is_protected: bool,
    pub is_static: bool,
    pub is_final: bool,
    pub is_abstract: bool,
    pub is_synchronized: bool,
    pub is_transient: bool,
    pub is_volatile: bool,
    pub is_native: bool,
    pub is_strictfp: bool,
    pub is_default: bool,
}

#[derive(Clone, Debug)]
pub struct Program {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Stmt { kind, span }
    }
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Empty,
    Block(Vec<Stmt>),
    Expr(Expr),
    VarDecl(VarDecl),
    Method(MethodDecl),
    Class(ClassDecl),
    If {
        cond: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
    },
    DoWhile {
        body: Box<Stmt>,
        cond: Expr,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        update: Vec<Expr>,
        body: Box<Stmt>,
    },
    ForEach {
        ty: Option<TypeRef>,
        name: String,
        iter: Expr,
        body: Box<Stmt>,
    },
    Switch {
        selector: Expr,
        arms: Vec<SwitchArm>,
    },
    Yield(Expr),
    Try {
        body: Box<Stmt>,
        catches: Vec<CatchClause>,
        finally: Option<Box<Stmt>>,
    },
    Throw(Expr),
    Assert {
        cond: Expr,
        message: Option<Expr>,
    },
    Return(Option<Expr>),
    Break(Option<String>),
    Continue(Option<String>),
    Labeled {
        label: String,
        body: Box<Stmt>,
    },
    Synchronized {
        lock: Expr,
        body: Box<Stmt>,
    },
    Import(ImportDecl),
    Package(String),
}

#[derive(Clone, Debug)]
pub struct ImportDecl {
    pub name: Vec<String>,
    pub is_static: bool,
    pub wildcard: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct SwitchArm {
    pub label: Option<Expr>,
    pub pattern: Option<SwitchPattern>,
    pub stmts: Vec<Stmt>,
    pub arrow: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct SwitchPattern {
    pub ty: TypeRef,
    pub name: Option<String>,
    pub components: Vec<(TypeRef, String)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct SwitchExprArm {
    pub label: Option<Expr>,
    pub pattern: Option<SwitchPattern>,
    pub body: SwitchExprBody,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum SwitchExprBody {
    Expr(Expr),
    Block(Vec<Stmt>),
}

#[derive(Clone, Debug)]
pub struct CatchClause {
    pub types: Vec<TypeRef>,
    pub name: String,
    pub body: Stmt,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct VarDecl {
    pub modifiers: Modifiers,
    /// None for a loosely typed BeanShell declaration (no type keyword).
    pub ty: Option<TypeRef>,
    pub vars: Vec<VarDeclarator>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct VarDeclarator {
    pub name: String,
    pub dims: usize,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    /// None for a loosely typed BeanShell parameter, e.g. "foo(a, b)".
    pub ty: Option<TypeRef>,
    pub name: String,
    pub varargs: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct MethodDecl {
    pub modifiers: Modifiers,
    pub return_type: Option<TypeRef>,
    pub name: String,
    pub params: Vec<Param>,
    pub throws: Vec<TypeRef>,
    pub body: Option<Box<Stmt>>,
    pub is_ctor: bool,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassKind {
    Class,
    Interface,
    Enum,
}

#[derive(Clone, Debug)]
pub struct EnumConstant {
    pub name: String,
    pub args: Vec<Expr>,
    pub body: Option<Vec<Stmt>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ClassDecl {
    pub modifiers: Modifiers,
    pub kind: ClassKind,
    pub name: String,
    pub extends: Option<TypeRef>,
    pub implements: Vec<TypeRef>,
    pub enum_constants: Vec<EnumConstant>,
    pub members: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { kind, span }
    }
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Literal(Literal),
    /// An ambiguous (dotted) name such as "a", "System.out" or "java.lang.String".
    Name(Vec<String>),
    /// Member access with an explicit base expression, e.g. (a + b).x
    Field(Box<Expr>, String),
    /// Null-safe member access: base?.name
    SafeField(Box<Expr>, String),
    /// Method reference: Type::method or Type::new
    MethodRef(Box<Expr>, String),
    /// Allocation of an inner class through an outer instance: outer.new Inner(...)
    NewInner {
        outer: Box<Expr>,
        ty: TypeRef,
        args: Vec<Expr>,
    },
    Index(Box<Expr>, Box<Expr>),
    /// Slice access: base[a:b:c]. Missing parts are None.
    Slice {
        base: Box<Expr>,
        parts: Vec<Option<Expr>>,
    },
    Call {
        target: Box<Expr>,
        args: Vec<Expr>,
    },
    NewArray {
        ty: TypeRef,
        dims: Vec<Option<Expr>>,
        init: Option<Vec<Expr>>,
    },
    New {
        ty: TypeRef,
        args: Vec<Expr>,
        anon: Option<Box<ClassDecl>>,
    },
    Cast(Box<Expr>, TypeRef),
    InstanceOf(Box<Expr>, TypeRef),
    InstanceOfPattern(Box<Expr>, TypeRef, String),
    SwitchExpr {
        selector: Box<Expr>,
        arms: Vec<SwitchExprArm>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Assign {
        op: AssignOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Ternary {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    /// The BeanShell elvis operator: "a ?: b" yields a when it is neither
    /// null nor false, otherwise b.
    Elvis {
        cond: Box<Expr>,
        else_expr: Box<Expr>,
    },
    ArrayInit(Vec<Expr>),
    This,
    Super,
    /// Lambda expression: parameters and body. The body is a block or a
    /// single expression statement.
    Lambda {
        params: Vec<Param>,
        body: Box<Stmt>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Byte(i8),
    Short(i16),
    Int(i64),
    /// An integer literal too large for i64 (BigInteger in BeanShell).
    BigInt(String),
    Long(i64),
    Float(f32),
    Double(f64),
    BigDec(String),
    Char(char),
    Str(String),
    Bool(bool),
    Null,
    /// The "void" literal used by BeanShell for untyped/undefined values.
    Void,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
    BitNot,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Ushr,
    /// BeanShell power operator: **
    Pow,
    /// BeanShell spaceship comparison: <=> (yields -1, 0 or 1)
    Cmp,
    /// BeanShell null coalescing operator: ??
    Coalesce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Ushr,
    Pow,
    Coalesce,
}
