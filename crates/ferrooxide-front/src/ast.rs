pub use crate::lexer::Number;

pub type Line = u32;

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    /// `timescale in effect for this module: (unit, precision) as powers of ten of seconds
    pub timescale: Option<(i32, i32)>,
    /// `default_nettype in effect ("wire", "wand", ..., "none"); None: wire
    pub default_nettype: Option<String>,
    /// `unconnected_drive pull1 (true) / pull0 (false): unconnected input ports are pulled
    pub unconnected_drive: Option<bool>,
    pub file: String,
    pub line: Line,
    /// Port names in header order.
    pub ports: Vec<String>,
    pub items: Vec<Item>,
    /// a user-defined primitive (`primitive ... endprimitive`): its table
    pub udp: Option<UdpDef>,
    /// names that are keywords of the keyword set in effect (an error in SystemVerilog; the
    /// lint check reports those that only IEEE 1800 reserves)
    pub keyword_names: Vec<(String, Line)>,
    /// an `interface ... endinterface`
    pub interface: bool,
    /// interface ports (`bus_if.master b`, `bus_if b[4]`): (port name, interface name, the
    /// range of an array port, the modport)
    pub iface_ports: Vec<(String, String, Option<Range>, Option<String>)>,
    /// modport names of an interface
    pub modports: Vec<String>,
    /// a package (or the file scope, `$unit`) holding variables: elaborated once, not a
    /// top-level module
    pub package: bool,
    /// packages (and `$unit`) whose variables the module sees after its own names
    pub imports: Vec<String>,
    /// a `program ... endprogram`
    pub program: bool,
    /// modules and programs declared inside this one (visible only here; those without
    /// ports are instantiated once, under their own name)
    pub nested: Vec<Module>,
    /// declared inside another module: names not found in it are looked up in the enclosing
    /// instance
    pub is_nested: bool,
}

/// A UDP table. Each row: input symbols ("0", "1", "x", "?", "b", "r", "f", "p", "n", "*" or an
/// edge like "(01)"), the current-state symbol for sequential UDPs, and the output ("0", "1",
/// "x", "-").
#[derive(Debug, Clone)]
pub struct UdpDef {
    pub sequential: bool,
    pub rows: Vec<(Vec<String>, Option<String>, String)>,
    /// `initial q = <value>;`
    pub init: Option<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Input,
    Output,
    Inout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetType {
    #[default]
    Wire,
    Tri0,
    Tri1,
    Supply0,
    Supply1,
    Wand,
    Wor,
    Trireg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetKind {
    Wire,
    Reg,
    Integer,
    Time,
    Genvar,
    /// SV `int`, `byte`, `shortint`, `longint`, `bit` (treated as 4-state for now)
    Int(u32),
    /// `real` / `realtime`
    Real,
    /// named `event`
    Event,
    /// SystemVerilog `string` (a handle of an interned string)
    String,
    Bit,
    /// a class handle (64 bits; `Decl::ty` names the class)
    Class,
}

pub type Range = (Expr, Expr);

#[derive(Debug, Clone)]
pub struct Declarator {
    pub name: String,
    pub dims: Vec<Range>,
    /// a queue or dynamic array (then `dims` is empty)
    pub heap: Option<HeapKind>,
    /// each element of the queue, dynamic or associative array `heap` is itself one of the
    /// first kind, whose elements are of the next, ... (`int a[string][$]`, `typedef int
    /// q_t[$]; q_t a[string];`)
    pub inner: Vec<HeapKind>,
    pub init: Option<Expr>,
    pub line: Line,
}

/// The unpacked dimension of a queue or dynamic array (its only one).
#[derive(Debug, Clone)]
pub enum HeapKind {
    /// `[$]`, `[$:max]`
    Queue(Option<Expr>),
    /// `[]`
    Dynamic,
    /// `[key_type]` associative array (`string` keys: kind `String`); None: `[*]`
    Assoc(Option<std::rc::Rc<crate::lower::SvType>>),
}

#[derive(Debug, Clone)]
pub struct Decl {
    pub dir: Option<Dir>,
    /// the net type of a `NetKind::Wire` declaration
    pub net: NetType,
    /// true if the kind was given explicitly (e.g. `output reg`) vs. implied
    pub explicit_kind: bool,
    /// `wire real`: a real-valued net
    pub real_net: bool,
    /// drive strength of a net declaration assignment (`wire (weak1, weak0) w = e;`)
    pub strength: Option<(u8, u8)>,
    /// net delay (`wire #5 w = e;`)
    pub delay: Option<Expr>,
    /// the range is written in the declaration (not that of a named type)
    pub range_written: bool,
    /// the declared data type; kind, sign and ranges (`kind()` & co.) are its flat form
    pub ty: std::rc::Rc<crate::lower::SvType>,
    pub names: Vec<Declarator>,
    /// declared `automatic` in a block: each activation (each instance of a fork branch) has
    /// its own copy
    pub automatic: bool,
    /// declared `static` in a block of a subroutine: one variable for all calls, its
    /// initializer applied once
    pub static_: bool,
    pub line: Line,
}

impl Decl {
    /// A variable of type `ty` (declared by the lowering, not in the source).
    pub fn var(ty: impl Into<std::rc::Rc<crate::lower::SvType>>, names: Vec<Declarator>, automatic: bool, line: Line) -> Decl {
        Decl {
            dir: None,
            net: NetType::Wire,
            explicit_kind: true,
            real_net: false,
            strength: None,
            delay: None,
            range_written: false,
            ty: ty.into(),
            names,
            automatic,
            static_: false,
            line,
        }
    }

    /// The kind (a struct type: a vector of `bit` or `logic`).
    pub fn kind(&self) -> NetKind {
        self.ty.decl_parts().0
    }

    pub fn set_kind(&mut self, k: NetKind) {
        std::rc::Rc::make_mut(&mut self.ty).kind = k;
    }

    pub fn signed(&self) -> bool {
        self.ty.signed
    }

    /// The first packed range (none for `int` & co.; a struct: its whole width).
    pub fn range(&self) -> Option<Range> {
        self.ty.decl_parts().2
    }

    /// Further packed dimensions (`reg [3:0][7:0] x`: range [3:0], packed [[7:0]]).
    pub fn packed(&self) -> &[Range] {
        self.ty.packed_rest()
    }

    /// Declared with a type parameter: its type (per instance) replaces kind, sign and ranges.
    pub fn type_param(&self) -> Option<&str> {
        self.ty.param.as_deref()
    }
}

#[derive(Debug, Clone)]
pub struct ParamDecl {
    pub local: bool,
    /// a `specparam` (not allowed in parameter values and generate conditions)
    pub spec: bool,
    /// a signed value (`parameter signed P = v` has no type of its own, only this)
    pub signed: bool,
    /// the declared data type; None: the value's type (IEEE 1800-2017 6.20.2)
    pub ty: Option<std::rc::Rc<crate::lower::SvType>>,
    /// the unpacked dimensions of an array parameter (`ty` is its element type)
    pub unpacked: Vec<Range>,
    pub name: String,
    /// None: a parameter port without a default, which every instance must override
    pub value: Option<Expr>,
    /// an enum constant: the constants of its enum (shared by the enum's type, `SvType::
    /// enum_names`, which makes them the enum's identity)
    pub enum_type: Option<std::rc::Rc<Vec<String>>>,
    pub line: Line,
}

impl ParamDecl {
    /// Of type `integer` or `int` (32 bits, signed, no range).
    pub fn integer(&self) -> bool {
        self.ty.as_ref().is_some_and(|t| t.int_kind && matches!(t.kind, NetKind::Integer | NetKind::Int(32)) && t.dims.len() <= 1)
    }

    pub fn real(&self) -> bool {
        self.ty.as_ref().is_some_and(|t| t.kind == NetKind::Real)
    }

    /// The range of the value: the declared one, the whole width of a struct or of several
    /// packed dimensions, that of `byte`, `shortint`, `longint` and `time`.
    pub fn range(&self) -> Option<Range> {
        // (an array parameter is no vector)
        if !self.unpacked.is_empty() {
            return None;
        }
        let t = self.ty.as_ref()?;
        if matches!(t.kind, NetKind::Real | NetKind::String) || self.integer() {
            return None;
        }
        if t.has_struct() || t.dims.len() > 1 {
            return Some((crate::lower::minus_one(t.width()), ExprKind::Num(dec(0)).into()));
        }
        // (a scalar `bit` or `logic`: one bit; a type parameter's type: the value's)
        if t.dims.is_empty() && t.param.is_none() && t.class.is_none() && matches!(t.kind, NetKind::Bit | NetKind::Reg) {
            return Some((ExprKind::Num(dec(0)).into(), ExprKind::Num(dec(0)).into()));
        }
        t.dims.first().cloned()
    }
}

#[derive(Debug, Clone)]
pub enum Conn {
    Positional(Option<Expr>),
    Named(String, Option<Expr>),
    /// SV `.*`
    Wildcard,
}

#[derive(Debug, Clone)]
pub struct Instance {
    pub name: String,
    pub dims: Vec<Range>,
    pub conns: Vec<Conn>,
    pub line: Line,
}

#[derive(Debug, Clone)]
pub struct SubroutineDecl {
    pub name: String,
    pub is_task: bool,
    pub automatic: bool,
    /// the return type (`void`: kind `Int(0)`; a task: `void`); `ret_kind()` & co. are its
    /// flat form
    pub ret_ty: std::rc::Rc<crate::lower::SvType>,
    /// ports in order + local declarations
    pub decls: Vec<Decl>,
    /// the `ref` ports (declared `Inout`: copied in and out, IEEE 1800-2017 13.5.2; the
    /// compiler rejects the calls where that differs from passing a reference)
    pub refs: Vec<String>,
    /// ports declared without a range that a later declaration gives one (`input a;
    /// reg [3:0] a;`): the line of that declaration (the reference warns about them)
    pub scalar_vec_ports: Vec<(String, Line)>,
    /// the port and type declarations as written, when `decls` merged some (for the checks
    /// of names declared twice); empty: `decls` are as written
    pub decls_written: Vec<Decl>,
    /// parameters declared in the subroutine (local to it)
    pub params: Vec<ParamDecl>,
    pub body: Stmt,
    pub line: Line,
}

impl SubroutineDecl {
    /// The kind of the result: `Int(0)` (none), `Integer` (32-bit signed), `Real`, `String`,
    /// `Class` or else `Reg` (a vector of `range()`).
    pub fn ret_kind(&self) -> NetKind {
        let t = &self.ret_ty;
        match t.kind {
            NetKind::Int(0) | NetKind::Real | NetKind::String | NetKind::Class if !t.has_struct() => t.kind,
            NetKind::Integer | NetKind::Int(32) if t.int_kind && t.dims.len() <= 1 && !t.has_struct() => NetKind::Integer,
            _ => NetKind::Reg,
        }
    }

    pub fn signed(&self) -> bool {
        self.ret_ty.signed
    }

    /// The range of a `Reg` result: none for one bit; a struct or several packed dimensions:
    /// the whole width.
    pub fn range(&self) -> Option<Range> {
        if self.ret_kind() != NetKind::Reg {
            return None;
        }
        let t = &self.ret_ty;
        if t.has_struct() || t.dims.len() > 1 {
            return Some((crate::lower::minus_one(t.width()), ExprKind::Num(dec(0)).into()));
        }
        t.dims.first().cloned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcKind {
    Initial,
    Always,
    AlwaysComb,
    AlwaysFf,
    AlwaysLatch,
    Final,
}

#[derive(Debug, Clone)]
pub enum Item {
    Decl(Decl),
    Param(ParamDecl),
    /// `parameter type T = logic [7:0];` (`default` None: no default type)
    TypeParam { local: bool, name: String, default: Option<std::rc::Rc<crate::lower::SvType>>, line: Line },
    /// a `specify` path delay: `(a, b *> q) = (rise, fall);` (conditions and edges ignored)
    /// `parallel`: a `=>` path between two whole terminals (the same width, 30.4.5)
    SpecPath { srcs: Vec<String>, dsts: Vec<String>, delay: Expr, parallel: bool, line: Line },
    /// `defparam a.b.P = value;` (the target path includes the parameter name)
    Defparam { target: Vec<String>, value: Expr, line: Line },
    /// `strength`: drive strength (strength0, strength1), levels 0 (highz) .. 7 (supply)
    ContAssign { delay: Option<Expr>, lhs: Expr, rhs: Expr, strength: Option<(u8, u8)>, line: Line },
    Process { kind: ProcKind, body: Stmt, line: Line },
    Instances { module: String, params: Vec<Conn>, insts: Vec<Instance>, line: Line },
    Subroutine(SubroutineDecl),
    Class(std::rc::Rc<ClassDecl>),
    /// `C #(args)` (IEEE 1800-2017 8.25): the specialization of parameterized class `class`
    /// with these parameter values (evaluated where it is used), named `name` (`C#<n>`) in
    /// the code that uses it
    ClassSpec { name: String, class: String, args: Vec<Conn>, line: Line },
    GenFor { init_var: String, init: Expr, cond: Expr, step_var: String, step: Expr, name: Option<String>, items: Vec<Item>, line: Line },
    /// `direct`: the whole block of an enclosing conditional (written without begin-end):
    /// its blocks are in the enclosing conditional's scope (IEEE 1800 27.5)
    GenIf { cond: Expr, then_items: Vec<Item>, then_name: Option<String>, else_items: Vec<Item>, else_name: Option<String>, direct: bool, line: Line },
    GenCase { expr: Expr, arms: Vec<(Vec<Expr>, Option<String>, Vec<Item>)>, direct: bool, line: Line },
    GenBlock { name: Option<String>, items: Vec<Item>, line: Line },
    /// gate primitive: and/or/nand/nor/xor/xnor/not/buf
    /// `dims`: an array of gate instances, e.g. `buf b[7:0] (o, i);`
    Gate {
        kind: String,
        inst_name: Option<String>,
        dims: Option<Range>,
        terms: Vec<Expr>,
        strength: Option<(u8, u8)>,
        delay: Option<Expr>,
        line: Line,
    },
    /// a concurrent assertion
    Assert(Box<ConcAssert>),
    /// an elaboration system task (IEEE 1800-2017 20.11): `$fatal`, `$error`, `$warning`,
    /// `$info` as an item, reported when its scope is elaborated
    ElabTask { name: String, args: Vec<Option<Expr>>, line: Line },
}

/// A concurrent assertion (IEEE 1800-2017 16.14): `assert property`, `assume property`,
/// `cover property` or `cover sequence`. Its expressions are evaluated on sampled values
/// (16.5.1) at the ticks of `clock`.
#[derive(Debug, Clone)]
pub struct ConcAssert {
    pub kind: AssertKind,
    /// the assertion's label: a scope for its action blocks
    pub label: Option<String>,
    /// the clocking event: the property's own, else that of the default clocking
    pub clock: EventCtl,
    /// `disable iff (e)` (or `default disable iff`): evaluated on current values
    pub disable: Option<Expr>,
    pub prop: Property,
    pub pass: Stmt,
    /// the fail action; the default reports an error
    pub fail: Stmt,
    pub line: Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertKind {
    Assert,
    Assume,
    /// `cover property`: non-vacuous successes run the pass action
    Cover,
    /// `cover sequence`: every match runs the pass action
    CoverSeq,
}

/// A property (IEEE 1800-2017 16.12).
#[derive(Debug, Clone)]
pub enum Property {
    /// holds when the sequence matches (weak: an attempt still running at the end passes)
    Seq(Sequence),
    Not(Box<Property>),
    And(Box<Property>, Box<Property>),
    Or(Box<Property>, Box<Property>),
    /// `ante |-> cons`, `ante |=> cons` (`next`)
    Implies { ante: Sequence, next: bool, cons: Box<Property> },
    /// `if (e) p [else q]`
    If(Expr, Box<Property>, Option<Box<Property>>),
}

/// A number of clock ticks `[lo:hi]` (`##`, repetitions): `hi` None is `lo` alone; `$` is
/// `unbounded`. The bounds are constant expressions.
#[derive(Debug, Clone)]
pub struct Cycles {
    pub lo: Expr,
    pub hi: Option<Expr>,
    pub unbounded: bool,
}

/// A sequence (IEEE 1800-2017 16.7-16.9) over the ticks of the assertion's clock.
#[derive(Debug, Clone)]
pub enum Sequence {
    /// a boolean expression for one tick (true: known nonzero)
    Bool(Expr),
    /// `a ##n b`; without `a` (`##n b`) the attempt's first tick is `1'b1`
    Delay(Option<Box<Sequence>>, Cycles, Box<Sequence>),
    /// `s[*n]`
    Repeat(Box<Sequence>, Cycles),
    /// `b[->n]`
    Goto(Expr, Cycles),
    /// `b[=n]`
    NonCons(Expr, Cycles),
    Or(Box<Sequence>, Box<Sequence>),
    And(Box<Sequence>, Box<Sequence>),
    Intersect(Box<Sequence>, Box<Sequence>),
    /// `b throughout s`
    Throughout(Expr, Box<Sequence>),
    /// `s within t`
    Within(Box<Sequence>, Box<Sequence>),
}

/// A class declaration. Inside its methods, the class's properties and methods (also
/// inherited ones) are reached through `this` (`ExprKind::Member(this, p)`,
/// `ExprKind::MethodCall(this, m, ..)`); `super` names `this` as the base class.
#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: String,
    /// the parameter ports `#(...)` of a parameterized class (`Item::Param` and
    /// `Item::TypeParam`): each specialization has its own values
    pub params: Vec<Item>,
    /// the specializations of parameterized classes the class uses (`Item::ClassSpec`,
    /// evaluated in its scope, so their arguments may use its parameters)
    pub specs: Vec<Item>,
    /// `extends B` and the arguments of `extends B(args)` (for B's constructor)
    pub base: Option<String>,
    pub base_args: Option<Vec<Expr>>,
    /// properties (each object has its own), with their initializers
    pub props: Vec<Decl>,
    /// the static properties of a parameterized class (`Item::Decl`): declared in the scope
    /// of each specialization, where its methods find them
    pub static_props: Vec<Item>,
    /// the typedefs declared in the class and its base classes (`C::T`)
    pub types: Vec<(String, std::rc::Rc<crate::lower::SvType>)>,
    /// parameters, local parameters and enum constants declared in the class (in its scope)
    pub consts: Vec<ParamDecl>,
    /// methods; a constructor is named `new`. Methods are automatic.
    pub methods: Vec<SubroutineDecl>,
    /// the methods declared `virtual`
    pub virtuals: Vec<String>,
    /// the methods declared `static` (no `this`)
    pub statics: Vec<String>,
    /// `virtual class`: no objects of its own
    pub abstract_: bool,
    /// properties and methods declared `local` (true) or `protected` (false)
    pub hidden: Vec<(String, bool)>,
    /// the `rand` properties (`randc`: true)
    pub rand: Vec<(String, bool)>,
    /// constraint blocks (IEEE 1800-2017 18.5); in their expressions the class's properties
    /// are reached through `this`
    pub constraints: Vec<Constraint>,
    pub line: Line,
}

/// `constraint name { items }`.
#[derive(Debug, Clone)]
pub struct Constraint {
    pub name: String,
    pub static_: bool,
    pub items: Vec<CItem>,
    pub line: Line,
}

/// A constraint item (IEEE 1800-2017 18.5).
#[derive(Debug, Clone)]
pub enum CItem {
    /// an expression that must be true (`soft`: unless it conflicts with others)
    Expr { soft: bool, e: Expr },
    /// `e inside {set}`: values and ranges `[lo:hi]`
    Inside { soft: bool, e: Expr, set: Vec<(Expr, Option<Expr>)> },
    /// `e dist {items}`
    Dist { soft: bool, e: Expr, items: Vec<DistItem> },
    /// `guard -> items`
    Implies(Expr, Vec<CItem>),
    /// `if (cond) items else items`
    If(Expr, Vec<CItem>, Vec<CItem>),
    /// `foreach (arr[i, j]) items`
    Foreach { arr: Expr, vars: Vec<Option<String>>, items: Vec<CItem> },
    /// `unique {values}`
    Unique(Vec<Expr>),
    /// `solve a before b`: it changes only how likely the solutions are
    SolveBefore,
    /// `disable soft e`
    DisableSoft(Expr),
}

/// A `dist` item: a value or range `[lo:hi]` with its weight (`per_value`: `:=`, each value
/// of the range has it; else `:/`, the range shares it).
#[derive(Debug, Clone)]
pub struct DistItem {
    pub lo: Expr,
    pub hi: Option<Expr>,
    pub per_value: bool,
    pub weight: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Any,
    Pos,
    Neg,
}

#[derive(Debug, Clone)]
pub enum EventCtl {
    Star,
    List(Vec<(Edge, Expr)>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    All,
    Any,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseKind {
    Case,
    CaseZ,
    CaseX,
}

#[derive(Debug, Clone)]
pub enum Timing {
    Delay(Expr),
    Event(EventCtl),
    /// intra-assignment `repeat (n) @(ev)`
    Repeat(Expr, EventCtl),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Null,
    /// `fork`: Some(join kind) for fork ... join / join_any / join_none
    /// `imports`: packages imported in the block (`P`: all its names, `P::x`: one name)
    /// `params`: parameters declared in the block (local to it)
    Block { name: Option<String>, decls: Vec<Decl>, imports: Vec<String>, params: Vec<ParamDecl>, stmts: Vec<Stmt>, fork: Option<Join> },
    Assign { lhs: Expr, rhs: Expr, nonblocking: bool, timing: Option<Timing>, line: Line },
    If { cond: Expr, then_s: Box<Stmt>, else_s: Option<Box<Stmt>>, line: Line },
    Case { kind: CaseKind, expr: Expr, arms: Vec<(Vec<Expr>, Stmt)>, default: Option<Box<Stmt>>, line: Line },
    For { init: Box<Stmt>, cond: Expr, step: Box<Stmt>, body: Box<Stmt>, line: Line },
    While { cond: Expr, body: Box<Stmt>, line: Line },
    DoWhile { cond: Expr, body: Box<Stmt>, line: Line },
    Repeat { count: Expr, body: Box<Stmt>, line: Line },
    Forever { body: Box<Stmt>, line: Line },
    Timed { timing: Timing, body: Box<Stmt>, line: Line },
    Wait { cond: Expr, body: Box<Stmt>, line: Line },
    SysTask { name: String, args: Vec<Option<Expr>>, line: Line },
    TaskCall { name: String, args: Vec<Expr>, line: Line },
    /// a method call as a statement (a task, or a function whose result is not used)
    MethodCall { obj: Expr, name: String, args: Vec<Expr>, line: Line },
    /// `C::m(args);`: a static method call as a statement
    StaticCall { class: String, name: String, args: Vec<Expr>, line: Line },
    Disable { name: String, line: Line },
    /// `force lhs = rhs` (`rhs` None: `release lhs`)
    Force { lhs: Expr, rhs: Option<Expr>, line: Line },
    Trigger { name: String, line: Line },
    Return { value: Option<Expr>, line: Line },
    Break { line: Line },
    Continue { line: Line },
    /// `void'(f(...))`: a function call whose value is not used; also an array method with
    /// `with` called as a statement (`q.sort with (e);`)
    Discard { e: Expr, line: Line },
    /// `wait fork`: until the fork branches the process started have ended
    WaitFork { line: Line },
    /// `foreach (array[i, j])` over an array the compiler resolves (a member, an element, a
    /// name declared elsewhere): one loop per named variable (None: a skipped dimension)
    Foreach { array: Expr, vars: Vec<Option<String>>, body: Box<Stmt>, line: Line },
    /// a deferred immediate assertion (IEEE 1800-2017 16.4): the condition is evaluated
    /// here, the action runs in the Reactive region (`#0`) or the Postponed region (`final`)
    /// of the time step, once for the last evaluation in it
    Deferred { final_: bool, cover: bool, cond: Expr, pass: Box<Stmt>, fail: Box<Stmt>, line: Line },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Plus,
    Neg,
    Not,
    LNot,
    RedAnd,
    RedOr,
    RedXor,
    RedNand,
    RedNor,
    RedXnor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    And,
    Or,
    Xor,
    Xnor,
    LAnd,
    LOr,
    Eq,
    Ne,
    CaseEq,
    CaseNe,
    WildEq,
    WildNe,
    Lt,
    Le,
    Gt,
    Ge,
    Shl,
    Shr,
    AShl,
    AShr,
}

/// An expression with the line it starts on (0: synthesized, no location of its own).
#[derive(Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: Line,
}

/// Only the expression itself: `{:?}` serves as a key of an expression in places, where
/// two equal expressions on different lines are the same.
impl std::fmt::Debug for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.kind.fmt(f)
    }
}

impl Expr {
    pub fn new(kind: ExprKind, line: Line) -> Expr {
        Expr { kind, line }
    }
}

impl Expr {
    /// Calls `f` for each direct subexpression (also those of patterns and of the constraints
    /// of `randomize() with`).
    pub fn for_each_child_mut(&mut self, f: &mut dyn FnMut(&mut Expr)) {
        use ExprKind as K;
        match &mut self.kind {
            K::Num(_) | K::Real(_) | K::Str(_) | K::Ident(_) | K::Hier(_) | K::Type(_) | K::Null | K::EmptyArg | K::Last | K::ClassMember { .. } => {}
            K::Index(a, b) | K::Binary(_, a, b) => {
                f(a);
                f(b);
            }
            K::Range(a, b, c) | K::IndexedRange(a, b, c, _) | K::Cond(a, b, c) => {
                f(a);
                f(b);
                f(c);
            }
            K::Unary(_, a) | K::Member(a, _) | K::NamedArg(_, a) | K::NewCopy(a) | K::IncDec { e: a, .. } => f(a),
            K::Concat(v) | K::Call(_, v) | K::SysCall(_, v) | K::New(v) | K::NewTyped(_, v) | K::StaticCall(_, _, v) => v.iter_mut().for_each(f),
            K::Stream { slice, items, .. } => {
                if let Some(s) = slice {
                    f(s);
                }
                items.iter_mut().for_each(f);
            }
            K::Repl(a, v) | K::MethodCall(a, _, v) => {
                f(a);
                v.iter_mut().for_each(f);
            }
            K::Pattern(_, items) => {
                for it in items {
                    match it {
                        PatItem::Pos(e) | PatItem::Key(_, e) | PatItem::Default(e) | PatItem::Type(_, e) => f(e),
                        PatItem::Index(a, b) => {
                            f(a);
                            f(b);
                        }
                        PatItem::Repl(n, v) => {
                            f(n);
                            v.iter_mut().for_each(&mut *f);
                        }
                    }
                }
            }
            K::NewArray(a, b) => {
                f(a);
                if let Some(b) = b {
                    f(b);
                }
            }
            K::ArrayWith { arr, with, .. } => {
                f(arr);
                f(with);
            }
            K::Randomize { obj, vars, with, .. } => {
                if let Some(o) = obj {
                    f(o);
                }
                vars.iter_mut().for_each(&mut *f);
                for it in with.iter_mut().flatten() {
                    it.for_each_expr_mut(f);
                }
            }
        }
    }

    /// Gives this expression and the subexpressions without a line of their own `line`.
    pub fn fill_line(&mut self, line: Line) {
        if self.line != 0 || line == 0 {
            return;
        }
        self.line = line;
        self.for_each_child_mut(&mut |c| c.fill_line(line));
    }
}

impl CItem {
    /// Calls `f` for each expression of the item (and of the items inside it).
    pub fn for_each_expr_mut(&mut self, f: &mut dyn FnMut(&mut Expr)) {
        match self {
            CItem::Expr { e, .. } | CItem::DisableSoft(e) => f(e),
            CItem::Inside { e, set, .. } => {
                f(e);
                for (a, b) in set {
                    f(a);
                    if let Some(b) = b {
                        f(b);
                    }
                }
            }
            CItem::Dist { e, items, .. } => {
                f(e);
                for d in items {
                    f(&mut d.lo);
                    if let Some(h) = &mut d.hi {
                        f(h);
                    }
                    f(&mut d.weight);
                }
            }
            CItem::Implies(g, items) => {
                f(g);
                items.iter_mut().for_each(|i| i.for_each_expr_mut(f));
            }
            CItem::If(c, a, b) => {
                f(c);
                a.iter_mut().chain(b.iter_mut()).for_each(|i| i.for_each_expr_mut(f));
            }
            CItem::Foreach { arr, items, .. } => {
                f(arr);
                items.iter_mut().for_each(|i| i.for_each_expr_mut(f));
            }
            CItem::Unique(v) => v.iter_mut().for_each(f),
            CItem::SolveBefore => {}
        }
    }
}

impl From<ExprKind> for Expr {
    /// an expression without a location (made by the compiler)
    fn from(kind: ExprKind) -> Expr {
        Expr { kind, line: 0 }
    }
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Num(Number),
    Real(f64),
    Str(String),
    Ident(String),
    /// hierarchical a.b.c
    Hier(Vec<String>),
    Index(Box<Expr>, Box<Expr>),
    /// [msb:lsb]
    Range(Box<Expr>, Box<Expr>, Box<Expr>),
    /// [base +: width] (up=true) or [base -: width]
    IndexedRange(Box<Expr>, Box<Expr>, Box<Expr>, bool),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Concat(Vec<Expr>),
    Repl(Box<Expr>, Vec<Expr>),
    Call(String, Vec<Expr>),
    SysCall(String, Vec<Expr>),
    /// a data type where a value can be a type (a type parameter's value)
    Type(std::rc::Rc<crate::lower::SvType>),
    /// a member of a struct value: `v.f`
    Member(Box<Expr>, String),
    /// an assignment pattern `'{...}` / `T'{...}`: resolved against its type, or without one
    /// against the type of the target it is assigned to
    Pattern(Option<std::rc::Rc<crate::lower::SvType>>, Vec<PatItem>),
    /// `new[size]`, `new[size](init)`: a dynamic array
    NewArray(Box<Expr>, Option<Box<Expr>>),
    /// `null`
    Null,
    /// an empty argument of a call (`f(a, , c)`): the argument's default value
    EmptyArg,
    /// `.name(e)` in a call: the argument for port `name` (`EmptyArg` for `.name()`)
    NamedArg(String, Box<Expr>),
    /// `new` / `new(args)`: an object of the class the target's type names
    New(Vec<Expr>),
    /// `C::new(args)`: an object of class C
    NewTyped(String, Vec<Expr>),
    /// `new obj`: a shallow copy of an object
    NewCopy(Box<Expr>),
    /// `obj.m(args)` (`this.m`, `super.m`): a method call
    MethodCall(Box<Expr>, String, Vec<Expr>),
    /// `C::m(args)`: a call of static method m of class C (also `m(args)` in C's methods)
    StaticCall(String, String, Vec<Expr>),
    /// `arr.m with (e)` (IEEE 1800-2017 7.12): array method `m` with `e` evaluated for each
    /// element; in `e` the element is `$it<depth>` and its index `$ix<depth>` (`depth`:
    /// of `with` expressions nested in one another, from 0)
    ArrayWith { arr: Box<Expr>, method: String, depth: usize, with: Box<Expr> },
    /// `obj.randomize(vars) with {constraints}` (IEEE 1800-2017 18.6, 18.7): obj None: of
    /// `this` (in a method), or `std::randomize(vars)` of variables (`std`); `vars`: the
    /// properties (variables) to randomize, all `rand` ones if empty; a 32-bit value, 1 on
    /// success
    Randomize { obj: Option<Box<Expr>>, vars: Vec<Expr>, with: Option<Vec<CItem>>, std: bool },
    /// `$` in an index of a queue: its last index
    Last,
    /// `++x` / `x--` in an expression: assigns `x` and gives the new (prefix) or old value
    IncDec { prefix: bool, inc: bool, e: Box<Expr> },
    /// `T::x` through a type parameter `T` (a class, known per specialization): a static
    /// property or a constant of the class (`T::f()` is a `StaticCall` of `T`)
    ClassMember { class: String, name: String },
    /// a streaming concatenation (IEEE 1800-2017 11.4.14): `{>> {items}}` (the items as a
    /// stream), `{<< slice {items}}` (blocks of `slice` bits, a constant or a type, from the
    /// right end in reverse order; default 1)
    Stream { left: bool, slice: Option<Box<Expr>>, items: Vec<Expr> },
}

/// An item of an assignment pattern.
#[derive(Debug, Clone)]
pub enum PatItem {
    Pos(Expr),
    /// `member: value` (or, for an array, an index named by a parameter)
    Key(String, Expr),
    /// `index: value` (an array element)
    Index(Expr, Expr),
    /// `default: value`
    Default(Expr),
    /// `type: value`: the members of that type (IEEE 1800-2017 10.9.2)
    Type(std::rc::Rc<crate::lower::SvType>, Expr),
    /// `n{a, b}`: the items repeated n times, a constant (10.10.1)
    Repl(Expr, Vec<Expr>),
}

/// An unsized decimal number.
pub fn dec(v: u64) -> Number {
    Number { size: None, signed: true, base: 10, based: false, digits: v.to_string(), fill: false }
}
