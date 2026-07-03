use crate::lexer::Span;

/// A parsed Lua expression with its byte span into the original source.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// The discriminated shape of an [`Expr`].
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Number(f64),
    /// ADR 0209 — integer-syntax literal preserved at AST level
    /// (Phase B of the ADR 0196 Integer/Float arc). HIR will lower
    /// to a matching `HirExprKind::Integer(i64)`; `infer_kind`
    /// returns `ValueKind::Number` for Phase B silent demotion so
    /// existing codegen + 125 `ValueKind::Number` consumers stay
    /// untouched. ADR 0210+ lifts the demotion at the kind layer.
    Integer(i64),
    /// String literal, escapes already processed by the lexer
    /// (Phase 2.7a, ADR 0024).
    Str(String),
    Bool(bool),
    Nil,
    Ident(String),
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    /// `function(params) body end` — anonymous function expression
    /// (Phase 2.5b, ADR 0017). Lowered to a `HirFunction` registered
    /// with the mangled name `user_anon_<idx>`.
    ///
    /// ADR 0293 — F1-A: `is_vararg` reflects a trailing `...` in the
    /// signature. HIR / codegen wiring lands in F1-B / F1-C; for
    /// F1-A the flag is captured but not yet consumed (HIR errors
    /// early with `VarargUnsupported`).
    FunctionExpr {
        params: Vec<String>,
        is_vararg: bool,
        body: Chunk,
    },
    /// ADR 0293 — F1-A: `...` in expression position. Represents
    /// the variadic pack visible inside a vararg function body.
    /// HIR / codegen wiring is F1-B / F1-C.
    Vararg,
    /// `{ field1, field2, … }` table constructor (Phase 2.6a-min,
    /// ADR 0053 for the empty form; ADR 0054 for the populated
    /// array form; ADR 0199 for keyed forms). Each field is one of
    /// the three Lua 5.4 §3.4.9 shapes — `Positional(expr)` for
    /// `e`, `Keyed { key: Str(name), value }` for `name = e`, and
    /// `Keyed { key, value }` for `[k] = e`. Trailing comma or
    /// semicolon allowed.
    Table(Vec<TableField>),
    /// `target[key]` array indexing (Phase 2.6a-arr, ADR 0054).
    /// `target` must be Table-kind, `key` Number-kind. Out-of-
    /// bounds reads trap at runtime.
    Index {
        target: Box<Expr>,
        key: Box<Expr>,
    },
    /// `recv:method(args)` method-call syntax (Phase 2.6+-methods,
    /// ADR 0092). Preserved AS-IS through the parser. HIR desugars
    /// at the chokepoint to `Call { callee: Index { recv, Str(method) },
    /// args: [recv, ...args] }`, materializing the receiver to a
    /// synthetic TaggedValue local exactly once when non-Ident
    /// (reuses the ADR 0091 `materialize_to_synth_local` helper).
    /// Receivers containing Call/MethodCall/FunctionExpr/BinOp/UnaryOp
    /// are rejected at HIR with `ComplexMethodReceiver`.
    MethodCall {
        receiver: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
}

/// ADR 0199 — one entry inside a `Table` constructor. Lua 5.4
/// §3.4.9 has three shapes; the named form `Name = exp` is
/// represented as `Keyed { key: Str(name), value }` so HIR
/// downstream sees the same shape for both keyed variants.
#[derive(Debug, Clone, PartialEq)]
pub enum TableField {
    Positional(Expr),
    Keyed { key: Expr, value: Expr },
}

/// Binary operators. Phase 2.2a covers all arithmetic operators
/// except `//` (floor div, deferred). Phase 2.2b adds the six relational
/// operators. Phase 2.3c adds the short-circuit logical operators.
/// Phase 2.2c (ADR 0022) adds floor div and the five bitwise operators.
/// See ADRs 0009, 0010, 0013, 0022.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    /// `//` floor division (Phase 2.2c, ADR 0022).
    FloorDiv,
    /// `&` bitwise AND (Phase 2.2c, ADR 0022).
    BitAnd,
    /// `|` bitwise OR (Phase 2.2c, ADR 0022).
    BitOr,
    /// `~` bitwise XOR (Phase 2.2c, ADR 0022). Distinguished from
    /// the unary `~` (`UnaryOp::BitNot`) by parser context.
    BitXor,
    /// `<<` left shift (Phase 2.2c, ADR 0022).
    Shl,
    /// `>>` arithmetic right shift (Phase 2.2c, ADR 0022).
    Shr,
    /// `..` string concatenation (Phase 2.7b, ADR 0025).
    /// Right-associative, between shift and additive in the
    /// precedence ladder.
    Concat,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

/// Unary prefix operators. Phase 2.2a introduces arithmetic negation.
/// Phase 2.3c adds logical `not`. Phase 2.2c (ADR 0022) adds bitwise
/// NOT (the unary `~`). Phase 2.7a (ADR 0024) adds the length
/// operator (the unary `#`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    /// `~x` bitwise complement (Phase 2.2c, ADR 0022).
    BitNot,
    /// `#s` string length in bytes (Phase 2.7a, ADR 0024).
    Len,
}

/// A statement. Phase 2.0 introduced `local` declarations and bare
/// expression statements; Phase 2.1 adds `Assign` and `Block` (do/end).
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Local {
        name: String,
        value: Expr,
        /// ADR 0236 — M9-A: Lua 5.4 §3.3.7 attribute on the
        /// declaration (`local x <const> = ...` /
        /// `local x <close> = ...`). `None` for an unadorned
        /// `local x`. HIR consumes this to set
        /// `LocalInfo::is_const` / `is_close`.
        attr: Option<String>,
    },
    /// `local NAME (, NAME)+ = EXPR (, EXPR)*` (Phase 2.5d, ADR 0021).
    /// `values.len() == 1` means a single Call expanded across all
    /// names; `values.len() == names.len()` means parallel binding;
    /// any other shape is a parse-time error.
    LocalMulti {
        names: Vec<String>,
        values: Vec<Expr>,
        /// ADR 0236 — per-name attribute aligned with `names`.
        /// `attrs.len() == names.len()`; entries default to `None`.
        attrs: Vec<Option<String>>,
    },
    Assign {
        name: String,
        value: Expr,
    },
    /// `target[key] = value` table element write (Phase 2.6a-wr,
    /// ADR 0055). `target` must lower to a Table-kind expression,
    /// `key` and `value` to Number-kind. Codegen emits a runtime
    /// bounds check (mirror of the read path); out-of-bounds traps
    /// rather than growing — Lua compatibility deferred until
    /// capacity tracking arrives.
    IndexAssign {
        target: Expr,
        key: Expr,
        value: Expr,
    },
    /// `NAME (, NAME)+ = EXPR (, EXPR)*` (Phase 2.1a, ADR 0049).
    /// Per Lua semantics every RHS is evaluated before any LHS
    /// is written, so `a, b = b, a` is a real swap. Lower layers
    /// require `values.len() == names.len()` (parallel binding) —
    /// multi-result Call expansion is the LocalMulti / 2.5d
    /// territory and is out of scope here.
    AssignMulti {
        names: Vec<String>,
        values: Vec<Expr>,
    },
    Block(Chunk),
    /// `if cond then ... [elseif cond then ...]* [else ...]? end`.
    /// `elifs` keeps the chain explicit (one entry per `elseif` arm).
    If {
        cond: Expr,
        then_body: Chunk,
        elifs: Vec<(Expr, Chunk)>,
        else_body: Option<Chunk>,
    },
    /// `while cond do ... end`.
    While {
        cond: Expr,
        body: Chunk,
    },
    /// `repeat ... until cond` (Phase 2.4b, ADR 0035). The body
    /// runs at least once; the cond is evaluated at the bottom and
    /// — per Lua 5.4 §3.3.4 — sees locals declared in the body.
    Repeat {
        body: Chunk,
        cond: Expr,
    },
    /// `for var = start, stop[, step] do body end` (Lua 5.4 §3.3.5).
    /// `step` is `None` when the implicit `1` is used.
    ForNumeric {
        var: String,
        start: Expr,
        stop: Expr,
        step: Option<Expr>,
        body: Chunk,
    },
    /// Phase 2.8e-iter-ipairs (ADR 0078): `for IDX, VAL in
    /// ipairs(TABLE) do BODY end` — restricted Lua iteration
    /// sugar. Only the `ipairs(table_expr)` shape is recognised
    /// in the iterator slot; arbitrary callable iterators (Lua's
    /// generic-for protocol) are parser-rejected today
    /// (LIC-2.8e-iter-generic-1). HIR desugars this variant to a
    /// synthetic `Block { LocalInit; While { LocalInit IndexTagged;
    /// IsNil break; BODY; idx += 1 } }` so codegen needs no new
    /// arm. `pairs(t)` is its sibling variant `ForPairs` (ADR 0080).
    ForIpairs {
        idx_name: String,
        val_name: String,
        table: Expr,
        body: Chunk,
    },
    /// `for K, V in pairs(TABLE) do BODY end` — Phase 2.8e-iter-pairs
    /// (ADR 0080) parser sugar. Only the `pairs(table_expr)` shape
    /// is recognised in the iterator slot. Unlike `ForIpairs`, this
    /// variant is preserved through HIR (as `HirStmtKind::ForPairs`)
    /// and lowered by a dedicated codegen walker — the hash-bucket
    /// traversal cannot be desugared to existing primitives.
    ForPairs {
        key_name: String,
        val_name: String,
        table: Expr,
        body: Chunk,
    },
    /// `for k, v in ITER, STATE, CTL do BODY end` — Phase 2.8e-iter-
    /// generic (ADR 0085) full Lua 5.4 §3.3.5 generic-for protocol.
    /// `iter` may be a builtin (`next`), a top-level user function,
    /// a function-typed local (parameter or alias), or — once ADR
    /// 0083 lands — a closure-with-upvalues. Phase 1 scope filters
    /// out closure-as-iter via the existing escape-analysis backstop
    /// (LIC-2.6c-tag-hetero-closure-escape-1). HIR desugars to a
    /// `Block { LocalInit __iter,__state,__ctl,_broken; While(true) {
    /// MultiAssignFromCall; If IsNil(k) then _broken=true else BODY;
    /// __ctl = k end } }` shape — no new codegen arm.
    ForGeneric {
        names: Vec<String>,
        iter: Expr,
        state: Expr,
        ctl: Expr,
        body: Chunk,
    },
    /// `break` — exits the innermost enclosing loop. HIR rejects
    /// `break` outside of any loop with `BreakOutsideLoop`.
    Break,
    /// `local function NAME(PARAMS) BODY end` (Phase 2.5a, ADR 0016).
    /// First-class anonymous functions arrive in 2.5b. ADR 0293 adds
    /// `is_vararg` for the parser-only trailing `...`.
    FunctionDef {
        name: String,
        params: Vec<String>,
        is_vararg: bool,
        body: Chunk,
    },
    /// `function recv.field(...) end` (`is_colon=false`) or
    /// `function recv:method(...) end` (`is_colon=true`); also
    /// supports multi-segment receiver chains since ADR 0096
    /// (`function a.b.c.method() end` / `function a.b.c:method() end`).
    /// `receiver_chain` carries the dotted chain head + intermediate
    /// segments; `method` is the final identifier. For single-segment
    /// ADR 0092 paths, `receiver_chain.len() == 1`. HIR
    /// `lower_method_def` folds the chain into a nested `Index` AST
    /// and emits `IndexAssign(target, Str(method), FunctionRef)`,
    /// reusing ADR 0095's `widen_index_for_assign_target` + TAG_TABLE
    /// runtime narrow for nested writes (length ≥ 2). Implicit
    /// `self` (kind Table per ADR 0092 MVP policy) is prepended to
    /// `params` when `is_colon`.
    MethodDef {
        receiver_chain: Vec<String>,
        method: String,
        is_colon: bool,
        params: Vec<String>,
        is_vararg: bool,
        body: Chunk,
    },
    /// `return [expr]`. HIR rejects `return` outside any function with
    /// `ReturnOutsideFunction`.
    Return {
        value: Option<Expr>,
    },
    /// `return EXPR, EXPR (, EXPR)*` — two or more return values
    /// (Phase 2.5d, ADR 0021).
    ReturnMulti {
        values: Vec<Expr>,
    },
    ExprStmt(Expr),
}

/// A Lua chunk — the top-level unit produced by [`super::parse`].
pub type Chunk = Vec<Stmt>;
