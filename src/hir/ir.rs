use crate::hir::ValueKind;
use crate::lexer::Span;
use crate::parser::{BinOp, UnaryOp};

/// Index into [`HirChunk::locals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

/// Per-local metadata. Phase 2.3a adds the static value kind that
/// determines the stack slot type at codegen time. Phase 2.5b.2 adds
/// `func_id`: when a Function-kind local was bound to a known
/// function (`local f = function() end` or alias), this carries that
/// `FuncId`; for function parameters whose value is only known at
/// runtime, it is `None` and the call site uses `Callee::Indirect`
/// (ADR 0018).
///
/// Phase 2.5c-full Commit 3 (ADR 0083) adds `is_captured`: set to
/// true when this local is referenced as `outer_local_id` of any
/// inner closure's `UpvalueInfo`. Codegen uses the flag to allocate
/// the slot as a heap upvalue box (so writes through the box are
/// visible to every closure sharing the same outer local). The
/// flag is filled by a post-pass after every function body has
/// been lowered.
/// ADR 0232 — M8-A: static subtype tag for Number-kind Locals.
/// `Integer` means the slot statically holds an integer (the
/// LocalInit / Assign RHS lowered to `HirExprKind::Integer` and
/// the slot has not been observably overwritten by a non-integer
/// expression). `Float` is the dual. `Unknown` means the slot
/// could carry either at runtime (e.g. assigned from a Local /
/// Call / param). Only applies when `kind == Number`; other
/// `ValueKind`s set this to `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberSubtype {
    Unknown,
    Integer,
    Float,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInfo {
    pub name: String,
    pub kind: ValueKind,
    pub func_id: Option<FuncId>,
    pub is_captured: bool,
    /// ADR 0232 — M8-A. See [`NumberSubtype`].
    pub subtype: NumberSubtype,
    /// ADR 0236 — M9-A: `<const>` attribute. When `true` the HIR
    /// rejects any Assign to this Local.
    pub is_const: bool,
}

/// A name-resolved program — the input to codegen.
///
/// `functions` carries every `local function` definition discovered at
/// the top level (Phase 2.5a; ADR 0016). `locals` and `stmts` describe
/// the implicit `main` chunk only.
#[derive(Debug, Clone, PartialEq)]
pub struct HirChunk {
    pub locals: Vec<LocalInfo>,
    pub stmts: Vec<HirStmt>,
    pub functions: Vec<HirFunction>,
}

/// Index into [`HirChunk::functions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncId(pub usize);

/// A user-defined function. The body is a fully-lowered statement
/// sequence with its own private `locals` table. Phase 2.5d (ADR
/// 0021) generalises the return type from `Option<ValueKind>` to a
/// `Vec<ValueKind>`. Phase 2.5c-min (ADR 0037) adds upvalues for
/// capture-by-value closures. Phase 2.5c-full Commit 3 (ADR 0083)
/// adds `parent_scope` so the post-pass that flips
/// `LocalInfo::is_captured` can resolve each upvalue's
/// `outer_local_id` to the right `locals` table.
#[derive(Debug, Clone, PartialEq)]
pub struct HirFunction {
    /// Source-level Lua name.
    pub name: String,
    /// MLIR symbol name — `user_<name>_<idx>`.
    pub mangled_name: String,
    /// Declared parameters in source order. Each is also the prefix of
    /// `locals` so that `LocalId(i)` for `i < params.len()` refers to
    /// the i-th parameter slot.
    pub params: Vec<LocalInfo>,
    /// Captured outer-scope locals (Phase 2.5c-min, ADR 0037).
    /// Filled by HIR upvalue analysis; codegen prepends them to the
    /// generated function's MLIR signature so direct call sites can
    /// pass the captured values as extra arguments.
    pub upvalues: Vec<UpvalueInfo>,
    /// All locals (params first, then upvalue-bound locals, then
    /// body-introduced locals + the synthetic `_returned` /
    /// `_ret_value_*` slots).
    pub locals: Vec<LocalInfo>,
    pub body: Vec<HirStmt>,
    /// Empty ⇒ void; length N ⇒ N return values, in source order.
    pub ret_kinds: Vec<ValueKind>,
    /// Phase 2.5c-full Commit 3 (ADR 0083): the lexical parent
    /// scope this function was declared in. `Chunk` for top-level
    /// functions and chunk-level anonymous functions; `Function(p)`
    /// for nested functions. Used by the `is_captured` post-pass to
    /// resolve each upvalue's `outer_local_id` to the correct
    /// scope's `locals` table.
    pub parent_scope: ParentScope,
}

/// Phase 2.5c-full Commit 3 (ADR 0083): identifies the lexical
/// parent of a [`HirFunction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentScope {
    Chunk,
    Function(FuncId),
}

/// One captured value for a closure (Phase 2.5c-min, ADR 0037).
/// `outer_local_id` is the LocalId in the enclosing scope where the
/// captured binding was declared; `inner_local_id` is where the
/// captured value lands inside this function's locals table.
/// Codegen emits the upvalue as an extra MLIR parameter after the
/// regular Lua params and stores the incoming block argument into
/// `slots[inner_local_id.0]` at function entry.
///
/// Capture is **by value**: the closure sees the outer slot's
/// content at the moment of each call (codegen reloads from the
/// caller's slot for `outer_local_id` and passes it as the extra
/// argument). This matches Lua's "upvalue is the binding" only
/// when the binding is never reassigned — Phase 2.5c-min users
/// either don't reassign captured locals or accept the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpvalueInfo {
    pub name: String,
    pub kind: ValueKind,
    pub outer_local_id: LocalId,
    pub inner_local_id: LocalId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirStmt {
    pub kind: HirStmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmtKind {
    LocalInit {
        id: LocalId,
        value: HirExpr,
    },
    Assign {
        id: LocalId,
        value: HirExpr,
    },
    Block {
        stmts: Vec<HirStmt>,
    },
    /// `if cond then ... [elseif cond then ...]* [else ...]? end`.
    /// Body, elif arms, and else body are independent lexical scopes.
    If {
        cond: HirExpr,
        then_body: Vec<HirStmt>,
        elifs: Vec<(HirExpr, Vec<HirStmt>)>,
        else_body: Option<Vec<HirStmt>>,
    },
    /// `while cond do body end`. `break_id` is `Some` when the body
    /// contains a reachable `break`; codegen AND-extends `cond` with
    /// `not load(break_slot)` in that case (ADR 0015).
    While {
        cond: HirExpr,
        body: Vec<HirStmt>,
        break_id: Option<LocalId>,
    },
    /// `repeat body until cond` (Phase 2.4b, ADR 0035). Same
    /// `break_id` treatment as `While`. The cond is lowered in the
    /// body's lexical scope per Lua 5.4 §3.3.4 — body-introduced
    /// locals are visible to the until-test. Codegen runs body +
    /// cond eval inside `scf.while`'s `before` region and continues
    /// while `not cond` (AND-extended with `not _broken` when the
    /// body holds a reachable `break`).
    Repeat {
        body: Vec<HirStmt>,
        cond: HirExpr,
        break_id: Option<LocalId>,
    },
    /// `for var = start, stop[, step] do body end` (Lua 5.4 §3.3.5).
    /// `step` is always present in the HIR — the parser's `Option`
    /// is materialised into a `HirExpr::Number(1.0)` at lowering time.
    /// `var_id` is the loop variable's slot, scoped to `body` only and
    /// recorded in `LowerCtx::readonly_locals` while the body lowers.
    /// `break_id` is `Some` when the body contains a reachable `break`;
    /// codegen AND-extends the natural cond with `not load(break_slot)`
    /// in that case. See ADR 0015.
    ForNumeric {
        var_id: LocalId,
        start: HirExpr,
        stop: HirExpr,
        step: HirExpr,
        body: Vec<HirStmt>,
        break_id: Option<LocalId>,
    },
    /// `local a, b, ... = f(args)` (Phase 2.5d, ADR 0021): a single
    /// multi-result call whose results are bound 1-1 to the listed
    /// destination locals. Equivalent in observable behaviour to
    /// "evaluate the call, then store each result into the matching
    /// `dst_ids[i]` slot", but represented atomically because codegen
    /// must emit the call once and read multiple `result(i)` values.
    MultiAssignFromCall {
        dst_ids: Vec<LocalId>,
        callee: Callee,
        args: Vec<HirExpr>,
    },
    /// `target[key] = value` table element write (Phase 2.6a-wr,
    /// ADR 0055). Mirror of `HirExprKind::Index` on the read side —
    /// codegen emits the same bounds-check + GEP, but stores
    /// `value` instead of loading.
    IndexAssign {
        target: HirExpr,
        key: HirExpr,
        value: HirExpr,
    },
    ExprStmt(HirExpr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExprKind {
    Number(f64),
    /// ADR 0209 — integer-syntax literal preserved at HIR. Phase B
    /// silent demotion: `infer_kind` returns `ValueKind::Number`
    /// so the 125-site `ValueKind::Number` consumer surface is
    /// untouched; codegen emits `arith::sitofp(i64, f64)` to keep
    /// downstream f64 paths working. ADR 0210+ lifts the demotion
    /// at the kind layer by introducing `ValueKind::Integer`.
    Integer(i64),
    /// String literal. Codegen materialises each unique payload as
    /// an `llvm.mlir.global` and emits an `addressof` at every use
    /// site. Phase 2.7a (ADR 0024).
    Str(String),
    Bool(bool),
    Nil,
    Local(LocalId),
    BinOp {
        op: BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<HirExpr>,
    },
    Call {
        callee: Callee,
        args: Vec<HirExpr>,
    },
    /// Reference to a user function by id (Phase 2.5b, ADR 0017).
    /// Produced by lowering an anonymous function expression
    /// `function() ... end` and stored into the matching Function-kind
    /// local; codegen treats it as an `i1 0` placeholder because the
    /// actual function is resolved by name at every call site.
    FunctionRef(FuncId),
    /// Table constructor (Phase 2.6a-min, ADR 0053; populated form
    /// in Phase 2.6a-arr, ADR 0054). Codegen mallocs
    /// `[length: i64][elem₀]...[elem_{N-1}]` (each elem 8 bytes
    /// for Number-only) and stores each element at offset
    /// `8 + i*8`.
    Table(Vec<HirExpr>),
    /// `target[key]` array indexing read (Phase 2.6a-arr, ADR 0054).
    /// `target` must be Table-kind, `key` Number-kind. Codegen
    /// emits a runtime bounds check that traps on out-of-bounds.
    Index {
        target: Box<HirExpr>,
        key: Box<HirExpr>,
    },
    /// Non-trapping tagged read of a table cell (Phase 2.6c-tag-
    /// locals, ADR 0063). Produced *only* by `lower_stmt(LocalInit
    /// | Assign)` when the source expression is
    /// `HirExprKind::Index`; codegen consumes it inline by
    /// `emit_local_init_tagged` to write `{tag, value}` into the
    /// local's 16-byte slot. Calling `emit_expr` on this variant
    /// is a programming error — use `Index` for value-context use.
    IndexTagged {
        target: Box<HirExpr>,
        key: Box<HirExpr>,
    },
    /// Non-trapping nil probe of a tagged-value source (Phase
    /// 2.6c-tag-hetero-eq, ADR 0066). Unifies the previous
    /// `IsNilQuery` (ADR 0061, `Index` operand) and `IsNilLocal`
    /// (ADR 0063, `Local(TaggedValue)` operand). Returns `Bool`
    /// — `true` when the source's runtime tag is Nil, `false`
    /// otherwise. The HIR pattern detection in
    /// `lower_expr::BinOp Eq/Ne` only generates two operand
    /// shapes: `Index { target, key }` (non-trapping table read)
    /// or `Local(LocalId)` with kind `TaggedValue` (slot tag
    /// check). Other operand shapes are unreachable.
    IsNil(Box<HirExpr>),
    /// Phase 2.7p-arith-string-coerce (ADR 0077): wraps a String
    /// operand of an arithmetic / bitwise BinOp with runtime
    /// numeric coercion. Codegen lowers via
    /// `emit_tonumber_for_arith` which traps on parse failure
    /// (Lua spec §3.4.1: `"abc" + 1` is a runtime error, not a
    /// silent NaN). The wrapped expression must have static kind
    /// `String`. Distinct from `Builtin::ToNumber` so the failure
    /// semantics differ: `tonumber("abc")` returns the NaN
    /// sentinel; arith coercion traps via
    /// `s_arith_coerce_failed`.
    ArithStringCoerce(Box<HirExpr>),
}

/// Discriminates whether a [`HirExprKind::Call`] hits a built-in
/// function (Phase 2.0 baseline), a statically-known user-defined
/// function (Phase 2.5a; ADR 0016), a runtime function value
/// reached through a Function-kind local — typically a parameter
/// (Phase 2.5b.2; ADR 0018) — or a static-candidate dispatch over
/// a TaggedValue local (Phase 2.5x-callee-dispatch; ADR 0082).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callee {
    Builtin(Builtin),
    /// Phase 2.5c-full Commit 3b (ADR 0083): user fn dispatch.
    /// `holding_local` carries the lexical binding through which
    /// this user fn was reached, when one exists. Codegen uses it
    /// to load the closure cell ptr from the right local slot —
    /// for capturing closures the cell ptr is stored in that slot
    /// (LocalInit storage rule). `None` only when the call
    /// resolved through `function_names` fallback (top-level
    /// forward reference / self-call inside a capturing fn body
    /// where the body's `lookup_or_capture_upvalue` rejects the
    /// Function-kind upvalue and falls through to the function-
    /// name table); codegen then uses the entry `cell_ptr`
    /// block-arg as the recursion shortcut.
    User {
        fid: FuncId,
        holding_local: Option<LocalId>,
    },
    /// Function-kind local whose statically-known arity (from
    /// `LocalInfo::kind`) reconstructs an `(...) → f64` signature.
    /// Today only function parameters reach this arm — every other
    /// Function local has a known FuncId and dispatches as `User`.
    Indirect(LocalId),
    /// Phase 2.5x-callee-dispatch (ADR 0082): a TaggedValue local
    /// (typically `local g = t[i]`) whose runtime value is one of
    /// `candidates` — the set of user functions whose signature
    /// matches `sig` (param + return kind vectors). Codegen emits
    /// a tag check, loads the payload as `!llvm.ptr`, and dispatches
    /// via per-candidate `if loaded == @user_fn_X then func.call
    /// @user_fn_X(args)` branches with `func.call` — no
    /// `func.call_indirect` cast (Codex pre-ADR-0082 review,
    /// forward-edge integrity).
    IndirectDispatch {
        local_id: LocalId,
        sig: IndirectSig,
        candidates: Vec<FuncId>,
    },
    /// ADR 0146 — Callable Table via `__call` metamethod. `t(args)`
    /// where `t` is a Table-kind Local. Codegen loads
    /// `t.metatable.__call` directly (not through `__index` chain
    /// per Lua spec §3.4.10) and dispatches with `(t, args...)` —
    /// the receiver Local becomes the implicit first arg `self`.
    /// `sig.param_kinds[0]` is `Table` (the self slot); subsequent
    /// kinds match the user-supplied args.
    TableCall {
        local_id: LocalId,
        sig: IndirectSig,
        candidates: Vec<FuncId>,
    },
}

/// Phase 2.5x-callee-dispatch (ADR 0082): the static signature
/// expected at an indirect call site. `compatible_user_functions`
/// filters the module's user functions to those whose `params` and
/// `ret_kinds` exactly match — full kind vectors, not just arity,
/// so multi-position TaggedValue ABIs (ADR 0076) stay unambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndirectSig {
    pub param_kinds: Vec<ValueKind>,
    pub ret_kinds: Vec<ValueKind>,
}

/// Recognised builtin functions. Phase 2.0 had only `print`; Phase
/// 2.7c (ADR 0026) added `tostring`; Phase 2.7e (ADR 0028) added
/// `tonumber`; Phase 2.7f (ADR 0029) adds `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Print,
    /// `tostring(x)` — converts Number/Bool/Nil/String to String.
    /// Function values are rejected (Phase 2.7c, ADR 0026).
    ToString,
    /// `tonumber(x)` — Number→identity, String→`sscanf("%lf")`
    /// with NaN sentinel on failure (Phase 2.7e, ADR 0028).
    /// Other kinds rejected.
    ToNumber,
    /// `type(x)` — returns the Lua type name as a String. Accepts
    /// every kind including Function values (the only builtin that
    /// does so; Phase 2.7f, ADR 0029).
    Type,
    /// `assert(v)` — passes the Bool unchanged on `true`, prints a
    /// fixed "assertion failed!" diagnostic and `exit(1)`s on
    /// `false`. Phase 2.7g (ADR 0030); the broader Lua signature
    /// (any kind, optional message arg, return value) is deferred.
    Assert,
    /// `error(msg)` — unconditional failure. Prints `msg` then
    /// `exit(1)`s. Phase 2.7h (ADR 0033); the optional `level`
    /// arg and table-as-message form are deferred.
    Error,
    /// `next(t, k)` — Lua spec §3.7.3 stateless hash-iteration step.
    /// Returns `(next_k, next_v)` where both are TaggedValue: the
    /// next non-nil entry after `k` in `t`'s iteration order, or
    /// `(nil, nil)` when the table is exhausted. The first builtin
    /// to declare a multi-position return signature (ADR 0081);
    /// `MultiAssignFromCall` is the only HIR shape that observes
    /// both result positions. Phase 2.8e-iter-next (ADR 0081).
    Next,
    /// `setmetatable(t, mt)` — Lua 5.4 §6.1 metatable installation.
    /// Stores `mt` at the table header's `metatable_ptr` slot
    /// (offset 32, introduced by ADR 0134's header growth). Returns
    /// `t` unchanged per Lua spec.
    ///
    /// Phase 2.6+-metatables-index-read (ADR 0134). The `nil` clear
    /// form (`setmetatable(t, nil)`) is out of scope — HIR enforces
    /// `[Table, Table]` and rejects nil at the second position.
    SetMetatable,
    /// `getmetatable(t)` — Lua 5.4 §6.1 metatable retrieval. Loads
    /// `metatable_ptr` from `t`'s header and returns it as a
    /// TaggedValue (Table when set, Nil when unset). The Lua spec
    /// `__metatable` field hiding is out of scope. Phase
    /// 2.6+-metatables-index-read (ADR 0134).
    GetMetatable,
    /// `rawset(t, k, v)` — Lua 5.4 §6.1 escape hatch from the
    /// `__newindex` metamethod chain. Writes `v` directly into `t`'s
    /// hash storage regardless of any metatable. Hash-key only —
    /// Number-key forms HIR-rejected per ADR 0136 (deferred with the
    /// array-OOB-widening ADR); Nil value HIR-rejected because
    /// `t[k] = nil` already bypasses `__newindex` via the ADR 0062
    /// tombstone path. Returns `t` per Lua spec.
    ///
    /// Phase 2.6+-raw-set-get-builtins (ADR 0136).
    RawSet,
    /// `rawget(t, k)` — Lua 5.4 §6.1 escape hatch from the `__index`
    /// metamethod chain. Reads `t`'s hash storage directly; returns
    /// the value at `t[k]` as TaggedValue, or Nil when the key is
    /// absent. Hash-key only per ADR 0136.
    ///
    /// Phase 2.6+-raw-set-get-builtins (ADR 0136).
    RawGet,
    /// `collectgarbage([opt])` — Lua 5.4 §6.1 GC entry point.
    /// ADR 0157 scope: 0-arg form runs a full collection (Phase 3
    /// no-op stub returning 0 until ADRs 0159 + 0161 land mark /
    /// sweep); 1-arg form must be `Str("count")` and returns
    /// `g_gc_total_bytes / 1024` as Number. Other options reject
    /// HIR-level.
    CollectGarbage,
    /// `rawequal(t1, t2)` — Lua 5.4 §6.1 escape hatch from the
    /// `__eq` metamethod chain. Today functionally identical to `==`
    /// (the `__eq` metamethod is deferred per ADR 0133); included
    /// for spec parity and forward-compat. Table operand only per
    /// ADR 0137 — String / Number / Bool / Function operand surface
    /// arrives with the `__eq` ADR.
    ///
    /// Phase 2.6+-raw-equal-len-builtins (ADR 0137).
    RawEqual,
    /// `rawlen(t)` — Lua 5.4 §6.1 escape hatch from the `__len`
    /// metamethod chain. Today functionally identical to `#t` (the
    /// `__len` metamethod is deferred per ADR 0133); included for
    /// spec parity and forward-compat. Table operand only per
    /// ADR 0137 — String operand arrives with the `__len` ADR.
    ///
    /// Phase 2.6+-raw-equal-len-builtins (ADR 0137).
    RawLen,
    /// `math.sqrt(x)` — square root via libm `sqrt`. Phase
    /// 2.7q-stdlib-math (ADR 0101). Dispatched at `lower_call` entry
    /// when the call shape is `Index{Ident("math"), Str("sqrt")}` AND
    /// `math` is an UNRESOLVED identifier (user shadowing is
    /// respected — `local math = ...` falls through to the normal
    /// Index-callee Call path).
    MathSqrt,
    /// `math.floor(x)` — floor (largest integer ≤ x) via libm
    /// `floor`. Same dispatch shape as `MathSqrt` (ADR 0101).
    MathFloor,
    /// `math.abs(x)` — absolute value via libm `fabs`. Same dispatch
    /// shape as `MathSqrt` (ADR 0101).
    MathAbs,
    /// `math.pow(x, y)` — exponentiation via libm `pow`. Phase
    /// 2.7q-stdlib-math (ADR 0102). The only binary math.* builtin
    /// today; arity=2 vs unary group=1.
    MathPow,
    /// `math.sin(x)` — sine via libm `sin` (ADR 0102).
    MathSin,
    /// `math.cos(x)` — cosine via libm `cos` (ADR 0102).
    MathCos,
    /// `math.log(x)` — natural logarithm via libm `log` (ADR 0102).
    /// Lua 5.4's optional second `base` arg is out of MVP scope.
    MathLog,
    /// `math.exp(x)` — exponential via libm `exp` (ADR 0102).
    MathExp,
    /// ADR 0240 — M11-A: `math.ceil(x)` smallest integer ≥ x via
    /// libm `ceil`. Same dispatch shape as MathFloor.
    MathCeil,
    /// ADR 0240 — `math.tan(x)` via libm `tan`.
    MathTan,
    /// ADR 0240 — `math.asin(x)` via libm `asin`.
    MathAsin,
    /// ADR 0240 — `math.acos(x)` via libm `acos`.
    MathAcos,
    /// ADR 0240 — `math.atan(x)` via libm `atan` (single-arg
    /// form only; two-arg `atan2` deferred).
    MathAtan,
    /// ADR 0241 — M11-B: `math.max(a, b, ...)` variadic max.
    /// Arity 1+, all Number args. Codegen reduces left-to-right
    /// via arith.maximumf.
    MathMax,
    /// ADR 0241 — `math.min(a, b, ...)` variadic min.
    MathMin,
    /// `string.len(s)` — Lua's `#s`-equivalent function-call form.
    /// Phase 2.7q-stdlib-string (ADR 0103). Dispatched via the
    /// generic namespace chokepoint when `string` is an UNRESOLVED
    /// identifier.
    StringLen,
    /// `string.upper(s)` — byte-wise ASCII uppercase. Allocates a
    /// new String via libc malloc + memcpy + char-loop with
    /// `toupper` (ADR 0103).
    StringUpper,
    /// `string.lower(s)` — byte-wise ASCII lowercase. Mirror of
    /// `StringUpper` with `tolower` (ADR 0103).
    StringLower,
    /// `string.sub(s, i [, j])` — Lua 5.4 §6.4 substring. The first
    /// range-arity (2-or-3) namespace builtin; arity-range check
    /// lives in `lower_namespace_builtin_call` per the `Assert`
    /// precedent in `lower_builtin_call` (`Builtin::arity()` reports
    /// the minimum, 2). Phase 2.7q-stdlib-string (ADR 0104).
    StringSub,
    /// `string.rep(s, n)` — Lua 5.4 §6.4 byte-wise repetition. Fixed
    /// arity 2; the variadic `sep` 3-arg form is a future ADR.
    /// `n <= 0` returns the empty string (runtime branch). Phase
    /// 2.7q-stdlib-string (ADR 0105).
    StringRep,
    /// `string.byte(s [, i])` — Lua 5.4 §6.4 single-position byte
    /// code retrieval. Returns the byte (0-255) at index i
    /// (default 1). Negative i indexes from the end. Out-of-range
    /// traps (Number-return contract has no nil representation;
    /// future multi-result-builtin ADR may restore nil semantics).
    /// 3rd consumer of ADR 0104's `emit_normalize_sub_bounds`
    /// helper (single-position trick: passes `j_raw = i_raw`).
    /// Phase 2.7q-stdlib-string (ADR 0109).
    StringByte,
    /// ADR 0228 — M7-A first sub-ADR: `string.find(s, sub)`
    /// literal substring search. Returns Number-or-Nil
    /// (TaggedValue) carrying the 1-indexed start position or
    /// Nil on no-match. Magic-char pattern semantics deferred;
    /// this entry handles only the "plain" case (no `%`, `*`,
    /// `+`, `?`, `[`, `]`, `(`, `)`, `^`, `$`, `.` etc).
    /// Multi-return (start, end) deferred to M7-B.
    StringFind,
    /// ADR 0230 — M7-C: `string.match(s, sub)` literal
    /// substring match. Returns the matched substring (String)
    /// on a hit or Nil on miss. In plain mode the matched
    /// substring equals `sub` itself; magic-char patterns +
    /// captures are deferred to the pattern matcher port.
    StringMatch,
    /// `string.char(...)` — Lua 5.4 §6.4 variadic byte-producer.
    /// Each arg must be an integer-valued Number in [0, 255];
    /// returns a boxed string object (ADR 0112). Embedded NUL
    /// is fully supported via the boxed ABI — the first new
    /// producer enabled by 0112.
    ///
    /// The first builtin with variadic Number per-position kind
    /// spec; introduces `Builtin::expected_param_kind(argc, pos)`
    /// because the existing static-slice `param_kinds_for_arity`
    /// API cannot represent argc-Number repetition.
    ///
    /// Validation chokepoint `emit_check_byte_arg` resolves the
    /// ADR 0105/0109 `emit_f2i` NaN/Inf carry-over for this site:
    /// range FIRST (cmpf Ord* rejects NaN/Inf naturally), integer
    /// SECOND (libm floor on finite x), f2i LAST (validated x).
    /// Phase 2.7v-stdlib-string-char (ADR 0113).
    StringChar,
    /// `string.format(fmt, ...)` — Lua 5.4 §6.4 printf-style
    /// formatter. ADR 0152 minimum-scope: format MUST be a static
    /// `Str` literal at the call site; supported specifiers are
    /// `%d` (Number → integer), `%f` (Number → float `%g`-style),
    /// `%s` (String → boxed-object data ptr), and `%%` (literal
    /// `%`). Width / precision / flag suffixes and other specifiers
    /// (`%e` / `%g` / `%i` / `%u` / `%x` / `%X` / `%o` / `%q` /
    /// `%c`) reject as `TypeMismatch` ("unsupported format spec").
    /// 16-arg cap. Phase 2.7q-stdlib-string-format (ADR 0152).
    StringFormat,
    /// `table.insert(list, [pos,] value)` — Lua 5.4 §6.8 array
    /// mutation. arity 2 (append) or 3 (positional insert with
    /// shift). The first **arity-sensitive** namespace builtin —
    /// arg 1 changes semantics: value (arity 2) vs pos (arity 3).
    /// Routed via `Builtin::param_kinds_for_arity(argc)` so the
    /// ADR 0110 policy contract stays uniform without
    /// re-introducing a `lower_namespace_builtin_call` special
    /// case. Phase 2.7r-stdlib-table (ADR 0111).
    TableInsert,
    /// `table.concat(t)` — Lua 5.4 §6.8 array-part concatenation
    /// with implicit `sep=""`. The first non-math, non-string
    /// consumer of ADR 0103's `Builtin::from_namespace_method`
    /// generic dispatcher. Fixed arity 1 (Option A scope); the
    /// 2/3/4-arg forms (sep / i / j) are deferred future ADRs.
    /// Elements must be Number or String; Bool/Nil/Table/Function
    /// trap at runtime per Lua spec. Phase 2.7r-stdlib-table
    /// (ADR 0106).
    TableConcat,
    /// `table.remove(list [, pos])` — Lua 5.4 §6.6 array
    /// mutation primitive. Mirror of ADR 0111 `TableInsert` with
    /// shift-LEFT instead of shift-right and a non-void
    /// TaggedValue return (the removed element).
    ///
    /// Validity: `1 ≤ pos ≤ #t` is normal; `pos == #t + 1` is a
    /// no-op returning nil; otherwise traps. Default `pos = #t`
    /// (tail pop) when arity is 1.
    ///
    /// First table.* builtin whose `ret_kinds` is non-empty —
    /// returns `[ValueKind::TaggedValue]` because the removed
    /// element can be any Lua kind. Phase 2.7r-stdlib-table
    /// (ADR 0118).
    TableRemove,
    /// `io.read([format])` — Lua 5.4 §6.6 line input. Reads from
    /// stdin via POSIX getline; returns the line as a String
    /// (without the trailing newline) or nil on EOF.
    ///
    /// MVP scope: format defaults to "*l" when omitted; explicit
    /// "*l" / "l" both accepted (Lua 5.3 deprecated `*` prefix
    /// but kept it). HIR rejects unsupported formats (`*n`, `*a`,
    /// `n`) and non-literal format args so the constraint fires
    /// at compile time. Phase 2.7x-stdlib-io-read (ADR 0119).
    ///
    /// Sibling of `IoWrite` in the io.* namespace. `ret_kinds =
    /// [TaggedValue]` because the result is either a String or
    /// nil; consumers see it via a tmp tagged slot per the ADR
    /// 0118 `emit_local_init_tagged` builtin-TaggedValue path.
    IoRead,
    /// `io.write(...)` — Lua 5.4 §6.6 stdout writer. Sibling of
    /// `print` without the tab separator and without a trailing
    /// newline. Variadic; each arg must be Number or String per
    /// the spec (Bool/Nil reject at HIR time, Function/Table
    /// reject via existing FunctionUsedAsValue walks).
    ///
    /// Phase 2.7x-stdlib-io-write (ADR 0116) — the first
    /// `io.*` namespace builtin and the 4th consumer of
    /// `Builtin::from_namespace_method`.
    IoWrite,
    /// ADR 0201 — `string.reverse(s)` byte-wise reversal.
    StringReverse,
    /// ADR 0210 — `math.type(x)` returns `"integer"` / `"float"`
    /// for source-syntax Integer / Number literals (and locals
    /// initialised from them). Source-other shapes (call-return,
    /// non-Number) return Nil per Lua 5.4 §6.7.
    MathType,
    /// ADR 0211 — `math.tointeger(x)` converts an exactly-
    /// representable integer-valued literal to its integer form
    /// (i64-as-f64 at Phase B); returns Nil for fractional
    /// literals and Locals / Calls / BinOp results. Phase C lifts
    /// the runtime case.
    MathToInteger,
    /// ADR 0216 — `pcall(f)` runs `f()` under a setjmp landing pad
    /// (ADR 0215). Returns `true` if `f` returned normally; `false`
    /// if any `error(msg)` longjmp'd back. The error message is
    /// discarded at this minimum-viable scope; ADR 0217 will add
    /// the second return value (`local ok, err = pcall(f)`) per the
    /// Lua 5.4 §6.1 multi-return contract. Phase B restriction:
    /// arg must be a Local of `ValueKind::Function(0)` (no-arg
    /// callable). Inline lambdas (`pcall(function() ... end)`) are
    /// rejected at HIR; users wrap in `local f = function...`.
    Pcall,
    /// ADR 0191 — Rust-Lua Bridge MVP demo function.
    /// `rust.add(a: Number, b: Number) -> Number` dispatches to
    /// `extern "C" fn rust_add(f64, f64) -> f64` in
    /// `src/bridge_runtime.rs`, linked into every produced
    /// binary via `build.rs` + `src/codegen/link.rs`. Extension
    /// surface: add a new `Builtin::Rust*` variant + one
    /// `rust_from_method` arm + one `extern "C" fn` in
    /// `src/bridge_runtime.rs`.
    RustAdd,
    /// ADR 0224 — Rust-Lua Bridge String → Number demo.
    /// `rust.strlen(s: String) -> Number` dispatches to
    /// `extern "C" fn rust_strlen(*const u8) -> f64` in
    /// `src/bridge_runtime.rs`. Reads the boxed-string-object
    /// length field (i64 at offset 0 per ADR 0112) and returns
    /// it as f64. First sub-ADR of the M5 marshaling milestone.
    RustStrlen,
    /// ADR 0225 — Rust-Lua Bridge Bool ↔ Bool demo.
    /// `rust.not(b: Bool) -> Bool` dispatches to
    /// `extern "C" fn rust_not(bool) -> bool` in
    /// `src/bridge_runtime.rs`. Second sub-ADR of the M5
    /// marshaling milestone.
    RustNot,
    /// ADR 0226 — mixed-kind multi-arg marshaling demo.
    /// `rust.starts_with(s: String, prefix: String) -> Bool`
    /// dispatches to
    /// `extern "C" fn rust_starts_with(*const u8, *const u8) -> bool`
    /// in `src/bridge_runtime.rs`. Third sub-ADR of the M5
    /// marshaling milestone.
    RustStartsWith,
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "print" => Some(Builtin::Print),
            "tostring" => Some(Builtin::ToString),
            "tonumber" => Some(Builtin::ToNumber),
            "type" => Some(Builtin::Type),
            "assert" => Some(Builtin::Assert),
            "error" => Some(Builtin::Error),
            // ADR 0216 — protected call.
            "pcall" => Some(Builtin::Pcall),
            "next" => Some(Builtin::Next),
            // ADR 0134 — metatables global builtins.
            "setmetatable" => Some(Builtin::SetMetatable),
            "getmetatable" => Some(Builtin::GetMetatable),
            // ADR 0136 — raw set / get hash-key escape hatches.
            "rawset" => Some(Builtin::RawSet),
            "rawget" => Some(Builtin::RawGet),
            // ADR 0137 — raw equal / len Lua spec parity.
            "rawequal" => Some(Builtin::RawEqual),
            "rawlen" => Some(Builtin::RawLen),
            // ADR 0157 — Phase 3 GC step 1: collectgarbage builtin.
            "collectgarbage" => Some(Builtin::CollectGarbage),
            _ => None,
        }
    }

    /// Phase 2.7q-stdlib-math (ADR 0101): map a `math.<method>` name
    /// to a Builtin variant. Returns None for unrecognized methods
    /// so `lower_call` falls through to the normal Index-callee
    /// path (which surfaces UndefinedName for unresolved `math`).
    pub fn math_from_method(method: &str) -> Option<Self> {
        match method {
            "sqrt" => Some(Builtin::MathSqrt),
            "floor" => Some(Builtin::MathFloor),
            "abs" => Some(Builtin::MathAbs),
            // ADR 0240 — M11-A math expansion.
            "ceil" => Some(Builtin::MathCeil),
            "tan" => Some(Builtin::MathTan),
            "asin" => Some(Builtin::MathAsin),
            "acos" => Some(Builtin::MathAcos),
            "atan" => Some(Builtin::MathAtan),
            // ADR 0241 — M11-B variadic max/min.
            "max" => Some(Builtin::MathMax),
            "min" => Some(Builtin::MathMin),
            // ADR 0102 — math.* continuation.
            "pow" => Some(Builtin::MathPow),
            "sin" => Some(Builtin::MathSin),
            "cos" => Some(Builtin::MathCos),
            "log" => Some(Builtin::MathLog),
            "exp" => Some(Builtin::MathExp),
            // ADR 0210 — math.type(x) returns "integer"/"float"
            // for static-kind Integer/Number literals; runtime
            // tag dispatch is future work.
            "type" => Some(Builtin::MathType),
            // ADR 0211 — math.tointeger(x) returns the integer
            // form of an exactly-representable Integer / Number
            // literal; nil otherwise.
            "tointeger" => Some(Builtin::MathToInteger),
            _ => None,
        }
    }

    /// Phase 2.7q-stdlib-string (ADR 0103): map a `string.<method>`
    /// name to a Builtin variant. Returns None for unrecognized
    /// methods so `lower_call` falls through to the normal
    /// Index-callee path.
    pub fn string_from_method(method: &str) -> Option<Self> {
        match method {
            "len" => Some(Builtin::StringLen),
            "upper" => Some(Builtin::StringUpper),
            "lower" => Some(Builtin::StringLower),
            // ADR 0104 — string.sub.
            "sub" => Some(Builtin::StringSub),
            // ADR 0105 — string.rep.
            "rep" => Some(Builtin::StringRep),
            // ADR 0109 — string.byte (single-position form).
            "byte" => Some(Builtin::StringByte),
            // ADR 0228 — M7-A literal-only string.find.
            "find" => Some(Builtin::StringFind),
            // ADR 0230 — M7-C literal-only string.match.
            "match" => Some(Builtin::StringMatch),
            // ADR 0113 — string.char (variadic byte-producer).
            "char" => Some(Builtin::StringChar),
            "format" => Some(Builtin::StringFormat),
            // ADR 0201 — string.reverse byte-wise.
            "reverse" => Some(Builtin::StringReverse),
            _ => None,
        }
    }

    /// Phase 2.7r-stdlib-table (ADR 0106): map a `table.<method>`
    /// name to a Builtin variant. Returns None for unrecognized
    /// methods so `lower_call` falls through to the normal
    /// Index-callee path.
    pub fn table_from_method(method: &str) -> Option<Self> {
        match method {
            "concat" => Some(Builtin::TableConcat),
            // ADR 0111 — table.insert (mutation primitive).
            "insert" => Some(Builtin::TableInsert),
            // ADR 0118 — table.remove (mutation primitive, mirror).
            "remove" => Some(Builtin::TableRemove),
            _ => None,
        }
    }

    /// Phase 2.7x-stdlib-io-write (ADR 0116): map an
    /// `io.<method>` name to a Builtin variant. First io.*
    /// builtin; future entries (read/open/close/...) extend
    /// here without touching the call-site dispatcher.
    pub fn io_from_method(method: &str) -> Option<Self> {
        match method {
            "write" => Some(Builtin::IoWrite),
            // ADR 0119 — io.read (line input).
            "read" => Some(Builtin::IoRead),
            _ => None,
        }
    }

    /// Phase 2.7q-stdlib-string (ADR 0103): generic namespace
    /// dispatcher. `lower_call`'s namespace builtin chokepoint
    /// invokes this with the unresolved `ns` identifier and the
    /// method name. Future ADRs (io.* etc.) extend here without
    /// touching the call site. ADR 0106 added the `table`
    /// namespace — the first non-math, non-string consumer.
    pub fn from_namespace_method(ns: &str, method: &str) -> Option<Self> {
        match ns {
            "math" => Self::math_from_method(method),
            "string" => Self::string_from_method(method),
            "table" => Self::table_from_method(method),
            // ADR 0116 — io.* namespace.
            "io" => Self::io_from_method(method),
            // ADR 0191 — Rust-Lua Bridge namespace.
            "rust" => Self::rust_from_method(method),
            _ => None,
        }
    }

    /// ADR 0191 — Rust-Lua Bridge: map a `rust.<method>` name to
    /// a Builtin variant. Returns None for unrecognized methods
    /// so `lower_call` falls through to the normal Index-callee
    /// path (which surfaces UndefinedName for unresolved `rust`,
    /// consistent with ADR 0103's math/string/table behaviour).
    pub fn rust_from_method(method: &str) -> Option<Self> {
        match method {
            "add" => Some(Builtin::RustAdd),
            // ADR 0224 — String → Number marshaling.
            "strlen" => Some(Builtin::RustStrlen),
            // ADR 0225 — Bool ↔ Bool marshaling demo. Method
            // name is `invert` because Lua's `not` is reserved.
            "invert" => Some(Builtin::RustNot),
            // ADR 0226 — mixed-kind multi-arg marshaling.
            "starts_with" => Some(Builtin::RustStartsWith),
            _ => None,
        }
    }

    /// Phase 2.7r-stdlib-table (ADR 0107): arity spec as
    /// `(min, max)` range. Fixed-arity builtins use `(N, N)`;
    /// range-arity builtins (Print, Assert, StringSub,
    /// TableConcat) report their inclusive bounds. Replaces the
    /// `arity() -> usize` + ad-hoc special-case branches at
    /// `lower_builtin_call` / `lower_namespace_builtin_call` that
    /// previously handled Print's variadic case and Assert /
    /// StringSub's range. Now both call sites do a uniform
    /// `args.len() < min || args.len() > max` check.
    pub fn arity(self) -> (usize, usize) {
        match self {
            // Variadic — Lua's `print` accepts any positive arity
            // including 0.
            Builtin::Print => (0, usize::MAX),
            Builtin::ToString => (1, 1),
            Builtin::ToNumber => (1, 1),
            Builtin::Type => (1, 1),
            // ADR 0030 / 0051: assert(v) or assert(v, msg).
            Builtin::Assert => (1, 2),
            Builtin::Error => (1, 1),
            // ADR 0216 — pcall(f) only at minimum scope; ADR 0217
            // widens to pcall(f, args...).
            Builtin::Pcall => (1, 1),
            // ADR 0198 — Lua 5.4 §6.1: next(table [, index]); arg 2 defaults to nil.
            Builtin::Next => (1, 2),
            // ADR 0134 — metatables global builtins.
            Builtin::SetMetatable => (2, 2),
            Builtin::GetMetatable => (1, 1),
            Builtin::RawSet => (3, 3),
            Builtin::RawGet => (2, 2),
            Builtin::RawEqual => (2, 2),
            Builtin::RawLen => (1, 1),
            // ADR 0157 — collectgarbage() / collectgarbage("count").
            // ADR 0200 — collectgarbage("setpause", n) → arity widens to (0, 2).
            Builtin::CollectGarbage => (0, 2),
            Builtin::MathSqrt | Builtin::MathFloor | Builtin::MathAbs => (1, 1),
            // ADR 0240 — unary math expansion sweep.
            Builtin::MathCeil
            | Builtin::MathTan
            | Builtin::MathAsin
            | Builtin::MathAcos
            | Builtin::MathAtan => (1, 1),
            // ADR 0241 — math.max / math.min variadic (1+).
            Builtin::MathMax | Builtin::MathMin => (1, usize::MAX),
            Builtin::MathPow => (2, 2),
            Builtin::MathSin | Builtin::MathCos | Builtin::MathLog | Builtin::MathExp => (1, 1),
            Builtin::StringLen | Builtin::StringUpper | Builtin::StringLower => (1, 1),
            // ADR 0104 — string.sub(s, i) or string.sub(s, i, j).
            Builtin::StringSub => (2, 3),
            // ADR 0105 — string.rep(s, n) exact arity 2.
            Builtin::StringRep => (2, 2),
            // ADR 0109 — string.byte(s [, i]) single-position
            // form. Multi-byte (s, i, j) is future work.
            Builtin::StringByte => (1, 2),
            // ADR 0228 — string.find(s, sub) — `init` and `plain`
            // args deferred to a future sub-ADR.
            Builtin::StringFind => (2, 2),
            Builtin::StringMatch => (2, 2),
            // ADR 0113 — string.char(...) variadic byte-producer.
            // Print precedent for variadic (0, usize::MAX).
            Builtin::StringChar => (0, usize::MAX),
            // ADR 0152 — string.format(fmt, ...) variadic. Format
            // arg is required (arity ≥ 1); 16-arg cap is enforced
            // at lower time rather than via arity bounds.
            Builtin::StringFormat => (1, 17),
            // ADR 0106/0107/0108 — table.concat(t [, sep [, i [, j]]]).
            // Lua 5.4 §6.8 full signature: arity 1 (default sep="",
            // i=1, j=#t), 2 (explicit sep), 3 (explicit i, default
            // j=#t), 4 (explicit i and j).
            Builtin::TableConcat => (1, 4),
            // ADR 0111 — table.insert(list, [pos,] value).
            // arity 2 = append, arity 3 = positional insert.
            Builtin::TableInsert => (2, 3),
            // ADR 0118 — table.remove(list [, pos]).
            // arity 1 = tail pop (default pos = #list);
            // arity 2 = explicit pos.
            Builtin::TableRemove => (1, 2),
            // ADR 0116 — io.write(...) variadic, Print precedent.
            Builtin::IoWrite => (0, usize::MAX),
            // ADR 0119 — io.read([format]). arity 0 = default
            // `"*l"`, arity 1 = explicit format ("*l" / "l").
            Builtin::IoRead => (0, 1),
            // ADR 0191 — rust.add(a, b) fixed arity 2.
            Builtin::RustAdd => (2, 2),
            // ADR 0224 — single-String-arg signature.
            Builtin::RustStrlen => (1, 1),
            Builtin::RustNot => (1, 1),
            Builtin::RustStartsWith => (2, 2),
            // ADR 0201 — string.reverse(s).
            Builtin::StringReverse => (1, 1),
            // ADR 0210 — math.type(x).
            Builtin::MathType => (1, 1),
            // ADR 0211 — math.tointeger(x).
            Builtin::MathToInteger => (1, 1),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Print => "print",
            Builtin::ToString => "tostring",
            Builtin::ToNumber => "tonumber",
            Builtin::Type => "type",
            Builtin::Assert => "assert",
            Builtin::Error => "error",
            Builtin::Pcall => "pcall",
            Builtin::Next => "next",
            Builtin::SetMetatable => "setmetatable",
            Builtin::GetMetatable => "getmetatable",
            Builtin::RawSet => "rawset",
            Builtin::RawGet => "rawget",
            Builtin::RawEqual => "rawequal",
            Builtin::RawLen => "rawlen",
            Builtin::CollectGarbage => "collectgarbage",
            Builtin::MathSqrt => "math.sqrt",
            Builtin::MathFloor => "math.floor",
            Builtin::MathAbs => "math.abs",
            Builtin::MathPow => "math.pow",
            Builtin::MathSin => "math.sin",
            Builtin::MathCos => "math.cos",
            Builtin::MathLog => "math.log",
            Builtin::MathExp => "math.exp",
            Builtin::MathCeil => "math.ceil",
            Builtin::MathTan => "math.tan",
            Builtin::MathAsin => "math.asin",
            Builtin::MathAcos => "math.acos",
            Builtin::MathAtan => "math.atan",
            Builtin::MathMax => "math.max",
            Builtin::MathMin => "math.min",
            Builtin::StringLen => "string.len",
            Builtin::StringUpper => "string.upper",
            Builtin::StringLower => "string.lower",
            Builtin::StringSub => "string.sub",
            Builtin::StringRep => "string.rep",
            Builtin::StringByte => "string.byte",
            Builtin::StringFind => "string.find",
            Builtin::StringMatch => "string.match",
            Builtin::StringChar => "string.char",
            Builtin::StringFormat => "string.format",
            Builtin::TableConcat => "table.concat",
            Builtin::TableInsert => "table.insert",
            Builtin::TableRemove => "table.remove",
            Builtin::IoWrite => "io.write",
            Builtin::IoRead => "io.read",
            // ADR 0191.
            Builtin::RustAdd => "rust.add",
            Builtin::RustStrlen => "rust.strlen",
            Builtin::RustNot => "rust.invert",
            Builtin::RustStartsWith => "rust.starts_with",
            // ADR 0201.
            Builtin::StringReverse => "string.reverse",
            // ADR 0210.
            Builtin::MathType => "math.type",
            // ADR 0211.
            Builtin::MathToInteger => "math.tointeger",
        }
    }

    /// Phase 2.8e-iter-next (ADR 0081): static return signature for a
    /// builtin call, used by `MultiAssignFromCall` lowering. Today
    /// every shipped builtin returns at most one value; the slot is
    /// here so future multi-return builtins (`next` in Commit 2,
    /// later `unpack` / `string.match` / etc.) can join the same
    /// dispatch. `Print` returns nothing — no value is observable
    /// from a `print(x)` call site.
    pub fn ret_kinds(self) -> &'static [ValueKind] {
        match self {
            Builtin::Print => &[],
            Builtin::ToString => &[ValueKind::String],
            Builtin::ToNumber => &[ValueKind::Number],
            Builtin::Type => &[ValueKind::String],
            Builtin::Assert => &[ValueKind::Bool],
            Builtin::Error => &[ValueKind::Number],
            // ADR 0216 — pcall single-return Bool.
            // ADR 0217 — pcall multi-return (Bool, TaggedValue).
            // Position 0 = ok flag (true = normal return / false =
            // caught error). Position 1 = err value (String message
            // captured from `error(msg)` / Nil on success). Single-
            // assign `local ok = pcall(f)` truncates to position 0
            // (Next precedent).
            Builtin::Pcall => &[ValueKind::Bool, ValueKind::TaggedValue],
            Builtin::Next => &[ValueKind::TaggedValue, ValueKind::TaggedValue],
            // ADR 0134 — setmetatable returns t (Table); getmetatable
            // returns the metatable (Table) or nil → TaggedValue.
            Builtin::SetMetatable => &[ValueKind::Table],
            Builtin::GetMetatable => &[ValueKind::TaggedValue],
            // ADR 0136 — rawset returns t (Table per Lua §6.1);
            // rawget returns t[k] or Nil → TaggedValue.
            Builtin::RawSet => &[ValueKind::Table],
            Builtin::RawGet => &[ValueKind::TaggedValue],
            // ADR 0137 — rawequal returns Bool; rawlen returns Number.
            Builtin::RawEqual => &[ValueKind::Bool],
            Builtin::RawLen => &[ValueKind::Number],
            // ADR 0157 — collectgarbage returns Number (count
            // value in KB for "count" form; 0 for no-op stub).
            Builtin::CollectGarbage => &[ValueKind::Number],
            Builtin::MathSqrt
            | Builtin::MathFloor
            | Builtin::MathAbs
            | Builtin::MathPow
            | Builtin::MathSin
            | Builtin::MathCos
            | Builtin::MathLog
            | Builtin::MathExp
            // ADR 0240 — M11-A unary sweep.
            | Builtin::MathCeil
            | Builtin::MathTan
            | Builtin::MathAsin
            | Builtin::MathAcos
            | Builtin::MathAtan
            // ADR 0241 — M11-B variadic max/min return Number.
            | Builtin::MathMax
            | Builtin::MathMin => &[ValueKind::Number],
            // Phase 2.7q-stdlib-string (ADR 0103/0109).
            Builtin::StringLen | Builtin::StringByte => &[ValueKind::Number],
            // ADR 0228 / 0229 — string.find multi-return
            // (start, end). Both TaggedValue Number-or-Nil.
            // Single-assign truncates to position 0 (start)
            // per ADR 0081 Next precedent.
            Builtin::StringFind => &[ValueKind::TaggedValue, ValueKind::TaggedValue],
            // ADR 0230 — string.match returns String-or-Nil
            // → TaggedValue.
            Builtin::StringMatch => &[ValueKind::TaggedValue],
            Builtin::StringUpper
            | Builtin::StringLower
            | Builtin::StringSub
            | Builtin::StringRep
            | Builtin::StringChar
            | Builtin::StringFormat
            | Builtin::TableConcat => &[ValueKind::String],
            // ADR 0111 — table.insert is void (Lua spec).
            Builtin::TableInsert => &[],
            // ADR 0118 — table.remove returns the removed element
            // as TaggedValue (table elements are heterogeneous).
            // First table.* with a non-void return.
            Builtin::TableRemove => &[ValueKind::TaggedValue],
            // ADR 0116 — io.write returns the file handle in the
            // Lua reference impl; MVP scope returns void
            // (Print precedent).
            Builtin::IoWrite => &[],
            // ADR 0119 — io.read returns a String on success or
            // nil on EOF; TaggedValue covers the union (TableRemove
            // precedent).
            Builtin::IoRead => &[ValueKind::TaggedValue],
            // ADR 0191 — rust.add returns Number.
            Builtin::RustAdd => &[ValueKind::Number],
            // ADR 0224 — returns Number (the i64 string length).
            Builtin::RustStrlen => &[ValueKind::Number],
            // ADR 0225 — rust.not returns Bool.
            Builtin::RustNot => &[ValueKind::Bool],
            // ADR 0226 — rust.starts_with returns Bool.
            Builtin::RustStartsWith => &[ValueKind::Bool],
            // ADR 0201 — string.reverse returns String.
            Builtin::StringReverse => &[ValueKind::String],
            // ADR 0210 — math.type returns "integer"/"float"/nil
            // → TaggedValue (String-or-Nil), matching IoRead /
            // GetMetatable precedent.
            Builtin::MathType => &[ValueKind::TaggedValue],
            // ADR 0211 — math.tointeger returns Number-or-Nil
            // (i64-as-f64 at Phase B; Phase C will use Integer
            // tagged value).
            Builtin::MathToInteger => &[ValueKind::TaggedValue],
        }
    }

    /// Phase 2.7t-stdlib-arg-kind-validation (ADR 0110): per-position
    /// expected kind for each argument of a namespace stdlib builtin
    /// (math.* / string.* / table.*). Slice length = max arity.
    /// Empty slice means "no per-arg checks" (global builtins are
    /// checked individually in `lower_builtin_call` and are out of
    /// scope for this ADR).
    ///
    /// `lower_namespace_builtin_call` (`src/hir/mod.rs`) iterates
    /// `args[0..args.len()]` against the returned slice — missing
    /// positions for range-arity builtins (Sub, Byte, Concat) are
    /// naturally skipped via `.get(i)`. TaggedValue args (table
    /// lookups, function params, etc.) are skipped at the call
    /// site; runtime tag-check is deferred to a future ADR.
    ///
    /// Phase 2.7r (ADR 0111): signature widened from
    /// `param_kinds()` to `param_kinds_for_arity(argc)` because
    /// `table.insert` is the first builtin where the per-position
    /// expected kind depends on the actual arity (arg 1 is the
    /// **value** in arity 2 vs the **pos** in arity 3). All other
    /// builtins ignore `argc` and return their static slice.
    pub fn param_kinds_for_arity(self, argc: usize) -> &'static [ValueKind] {
        match self {
            // Global builtins: not checked here (existing per-builtin
            // logic in `lower_builtin_call`). Future ADR may unify.
            Builtin::Print
            | Builtin::ToString
            | Builtin::ToNumber
            | Builtin::Type
            | Builtin::Assert
            | Builtin::Error
            // ADR 0216 — per-arg validation in `lower_builtin_call`.
            | Builtin::Pcall
            | Builtin::Next
            // ADR 0134 — global builtins; per-arg kind checks live
            // in `lower_builtin_call` alongside Assert / Next.
            | Builtin::SetMetatable
            | Builtin::GetMetatable
            // ADR 0136 — per-arg kind checks live in
            // `lower_builtin_call` alongside SetMetatable.
            | Builtin::RawSet
            | Builtin::RawGet
            // ADR 0137 — per-arg kind checks live in
            // `lower_builtin_call`.
            | Builtin::RawEqual
            | Builtin::RawLen
            // ADR 0157 — per-arg validation in `lower_builtin_call`.
            | Builtin::CollectGarbage => &[],
            // math.* — all single-Number arg except pow (Number, Number).
            Builtin::MathSqrt
            | Builtin::MathFloor
            | Builtin::MathAbs
            | Builtin::MathSin
            | Builtin::MathCos
            | Builtin::MathLog
            | Builtin::MathExp
            // ADR 0240 — unary math expansion.
            | Builtin::MathCeil
            | Builtin::MathTan
            | Builtin::MathAsin
            | Builtin::MathAcos
            | Builtin::MathAtan => &[ValueKind::Number],
            Builtin::MathPow => &[ValueKind::Number, ValueKind::Number],
            // string.* — first arg String; bounds args Number.
            Builtin::StringLen | Builtin::StringUpper | Builtin::StringLower => {
                &[ValueKind::String]
            }
            Builtin::StringSub => &[ValueKind::String, ValueKind::Number, ValueKind::Number],
            Builtin::StringRep => &[ValueKind::String, ValueKind::Number],
            Builtin::StringByte => &[ValueKind::String, ValueKind::Number],
            // ADR 0228 — string.find(s, sub).
            Builtin::StringFind => &[ValueKind::String, ValueKind::String],
            Builtin::StringMatch => &[ValueKind::String, ValueKind::String],
            // table.* — first arg Table; sep String; bounds Number.
            Builtin::TableConcat => &[
                ValueKind::Table,
                ValueKind::String,
                ValueKind::Number,
                ValueKind::Number,
            ],
            // ADR 0111 — table.insert is arity-sensitive:
            //   arity 2: [Table, TaggedValue]  (arg 1 = value, any kind)
            //   arity 3: [Table, Number, TaggedValue]  (arg 1 = pos)
            // Other argc values are unreachable in practice
            // (arity range (2, 3) check fires earlier in
            // `lower_namespace_builtin_call`).
            Builtin::TableInsert => match argc {
                2 => &[ValueKind::Table, ValueKind::TaggedValue],
                3 => &[ValueKind::Table, ValueKind::Number, ValueKind::TaggedValue],
                _ => &[],
            },
            // ADR 0118 — table.remove arity-sensitive:
            //   arity 1: [Table]                 (default pos = #t)
            //   arity 2: [Table, Number]         (explicit pos)
            Builtin::TableRemove => match argc {
                1 => &[ValueKind::Table],
                2 => &[ValueKind::Table, ValueKind::Number],
                _ => &[],
            },
            // ADR 0119 — io.read([format]). arity 0 = no args
            // (default "*l"); arity 1 = String literal format.
            // The literal-value check ("*l" / "l" only) lives
            // in `lower_namespace_builtin_call`.
            Builtin::IoRead => match argc {
                0 => &[],
                1 => &[ValueKind::String],
                _ => &[],
            },
            // ADR 0113 — string.char is variadic Number; the
            // static-slice API cannot express argc-Number
            // repetition. `expected_param_kind` handles this
            // builtin directly and bypasses the slice.
            Builtin::StringChar => &[],
            // ADR 0152 — string.format is variadic with per-arg
            // expected kind driven by the format specifier; HIR
            // lower drives validation directly. The slice stays
            // empty so the generic check is bypassed (Print
            // precedent — the per-arg check lives in
            // `lower_namespace_builtin_call`).
            Builtin::StringFormat => &[],
            // ADR 0116 — io.write is variadic Number-or-String;
            // standard slice check uses TaggedValue sentinel
            // (any-accepted), followed by a IoWrite-specific
            // Bool/Nil reject in `lower_namespace_builtin_call`.
            Builtin::IoWrite => &[],
            // ADR 0191 — rust.add(a, b) — both args Number.
            Builtin::RustAdd => &[ValueKind::Number, ValueKind::Number],
            Builtin::RustStrlen => &[ValueKind::String],
            Builtin::RustNot => &[ValueKind::Bool],
            Builtin::RustStartsWith => &[ValueKind::String, ValueKind::String],
            // ADR 0201 — string.reverse(s) — String arg.
            Builtin::StringReverse => &[ValueKind::String],
            // ADR 0210 — math.type accepts any Number-kind arg
            // (subtype distinction is shape-based at HIR).
            Builtin::MathType => &[ValueKind::Number],
            // ADR 0211 — math.tointeger accepts Number.
            Builtin::MathToInteger => &[ValueKind::Number],
            // ADR 0241 — variadic Number args; per-position
            // dispatched via `expected_param_kind`.
            Builtin::MathMax | Builtin::MathMin => &[ValueKind::Number],
        }
    }

    /// Phase 2.7v-stdlib-string-char (ADR 0113): per-position
    /// expected kind, given the call-site arity and position.
    /// Generalises `param_kinds_for_arity` to handle variadic
    /// per-position kind specs (`string.char` is the first
    /// trigger: every position is Number for any argc).
    ///
    /// Existing builtins delegate to `param_kinds_for_arity`
    /// (slice-indexed lookup) — zero behavioural change.
    /// `lower_namespace_builtin_call`'s check loop drives this
    /// method per position so variadic Number works uniformly.
    pub fn expected_param_kind(self, argc: usize, pos: usize) -> Option<ValueKind> {
        match self {
            // Every position is Number, regardless of argc.
            Builtin::StringChar => Some(ValueKind::Number),
            // ADR 0241 — math.max / math.min: every position is
            // Number, variadic 1+.
            Builtin::MathMax | Builtin::MathMin => Some(ValueKind::Number),
            // ADR 0152 — per-arg kind is specifier-driven; HIR
            // lower validates directly. Use the TaggedValue
            // sentinel here to skip the standard kind check.
            Builtin::StringFormat => Some(ValueKind::TaggedValue),
            // ADR 0116 — io.write accepts Number or String per
            // position. Use the TaggedValue sentinel to skip the
            // standard single-kind check; lower_namespace_builtin_call
            // adds an IoWrite-specific Number/String-only loop
            // that rejects Bool/Nil.
            Builtin::IoWrite => Some(ValueKind::TaggedValue),
            _ => self.param_kinds_for_arity(argc).get(pos).copied(),
        }
    }
}
