//! The syntax tree: what the source says, nothing resolved (IEEE 1800-2017 Annex A).
//!
//! Names are plain strings; whether `a.b` is a hierarchical name or a member, or `p::x` a
//! package item or a class member, is decided later (lowering, elaboration). Lists keep
//! source order. `Loc` is the line of the construct's first token.

pub use crate::lexer::Number;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Loc {
    pub line: u32,
}

/// One source file (after preprocessing).
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub name: String,
    pub items: Vec<Description>,
}

#[derive(Clone, Debug)]
pub enum Description {
    /// module, macromodule, interface, program, checker
    Module(ModuleDecl),
    Udp(UdpDecl),
    Package(PackageDecl),
    Config(ConfigDecl),
    /// a file-scope (`$unit`) item: declarations, typedefs, subroutines, classes, imports,
    /// `bind`, ...
    Item(Item),
    Directive(Directive, Loc),
}

/// Compiler directives the preprocessor passes on (in source order).
#[derive(Clone, Debug, PartialEq)]
pub enum Directive {
    /// unit, precision (powers of ten of seconds)
    Timescale(i32, i32),
    /// `default_nettype (the net type or "none")
    DefaultNettype(String),
    /// `unconnected_drive pull1 (Some(true)) / pull0 / `nounconnected_drive (None)
    UnconnectedDrive(Option<bool>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleKind {
    Module,
    Macromodule,
    Interface,
    Program,
    Checker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    Static,
    Automatic,
}

#[derive(Clone, Debug)]
pub struct ModuleDecl {
    pub kind: ModuleKind,
    pub name: String,
    pub lifetime: Option<Lifetime>,
    pub extern_: bool,
    /// `import p::*;` in the header
    pub imports: Vec<Import>,
    /// `#( ... )`: None without one
    pub params: Option<Vec<ParamDecl>>,
    pub ports: Ports,
    pub items: Vec<Item>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum Ports {
    /// no port list
    None,
    /// `(a, b[3:0], .c(x), {d, e}, )`: port expressions (an empty entry is an unconnected port)
    NonAnsi(Vec<PortExpr>),
    /// `(input logic a, output b, bus_if.mp c)`
    Ansi(Vec<AnsiPort>),
    /// `(.*)` (checkers, extern modules)
    Wildcard,
}

#[derive(Clone, Debug)]
pub struct PortExpr {
    /// `.name(...)`: the explicit external name
    pub name: Option<String>,
    /// the port expression (None: empty)
    pub expr: Option<Expr>,
    pub loc: Loc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Input,
    Output,
    Inout,
    Ref,
    /// `const ref`
    ConstRef,
}

/// An ANSI port as written: what is omitted is inherited from the previous port (IEEE 1800
/// 23.2.2.3), which the lowering applies.
#[derive(Clone, Debug)]
pub struct AnsiPort {
    pub dir: Option<Dir>,
    pub header: PortHeader,
    pub name: String,
    pub dims: Vec<Dim>,
    /// a default value (`input int a = 1`)
    pub default: Option<Expr>,
    /// `.name(expr)` explicit port
    pub expr: Option<Option<Expr>>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum PortHeader {
    /// nothing but the name (everything inherited)
    None,
    /// a net type and/or data type: `wire logic [3:0]`, `var int`, `[7:0]`, `signed`
    Data { net: Option<NetTypeKw>, var: bool, ty: DataType },
    /// interface port: `bus_if b`, `bus_if.master b`, `interface b`, `interface.mp b`
    Interface { name: Option<String>, modport: Option<String> },
}

// ---------------------------------------------------------------- types

#[derive(Clone, Debug)]
pub struct DataType {
    pub kind: TypeKind,
    /// `signed` / `unsigned` (None: not given)
    pub signed: Option<bool>,
    /// packed dimensions after the type
    pub packed: Vec<Dim>,
    pub loc: Loc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntKw {
    Bit,
    Logic,
    Reg,
    Byte,
    Shortint,
    Int,
    Longint,
    Integer,
    Time,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealKw {
    Real,
    Realtime,
    Shortreal,
}

#[derive(Clone, Debug)]
pub enum TypeKind {
    /// no type keyword (only signing and/or packed dimensions, or nothing)
    Implicit,
    Int(IntKw),
    Real(RealKw),
    String,
    Chandle,
    Event,
    Void,
    Struct(Box<StructType>),
    Enum(Box<EnumType>),
    /// a type name, possibly scoped (`p::T`, `C#(int)::T`, `$unit::T`)
    Named(Box<Name>),
    /// `virtual [interface] bus_if #(...) .mp`
    VirtualInterface { name: String, params: Option<Vec<ParamArg>>, modport: Option<String> },
    /// `type(expr)` / `type(data_type)`
    TypeOf(Box<Expr>),
}

#[derive(Clone, Debug)]
pub struct StructType {
    pub union: bool,
    pub tagged: bool,
    /// `union soft`
    pub soft: bool,
    pub packed: bool,
    pub members: Vec<StructMember>,
}

#[derive(Clone, Debug)]
pub struct StructMember {
    /// `rand` / `randc`
    pub rand: Option<Kw>,
    pub ty: DataType,
    pub decls: Vec<Declarator>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct EnumType {
    pub base: Option<DataType>,
    pub items: Vec<EnumItem>,
}

#[derive(Clone, Debug)]
pub struct EnumItem {
    pub name: String,
    /// `name[N]` / `name[N:M]`: a range of names
    pub range: Option<(Expr, Option<Expr>)>,
    pub value: Option<Expr>,
    pub loc: Loc,
}

use super::kw::Kw;

/// A dimension, packed or unpacked.
#[derive(Clone, Debug)]
pub enum Dim {
    /// `[a:b]`
    Range(Expr, Expr),
    /// `[n]` (unpacked: `[0:n-1]`)
    Size(Expr),
    /// `[]` dynamic array
    Unsized,
    /// `[$]` / `[$:max]` queue
    Queue(Option<Expr>),
    /// `[*]` (None) or `[type]` associative array
    Assoc(Option<DataType>),
}

// ---------------------------------------------------------------- names

/// A possibly scoped, possibly hierarchical name used as a type or as a scope prefix:
/// `T`, `p::T`, `C#(int)::T`, `$unit::x`, `local::x`.
#[derive(Clone, Debug)]
pub struct Name {
    /// scope parts before the last `::`
    pub scopes: Vec<NamePart>,
    pub name: String,
    /// `C#(int)`: parameter values of a class type
    pub params: Option<Vec<ParamArg>>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct NamePart {
    pub name: String,
    /// class parameter values `C#(...)::`
    pub params: Option<Vec<ParamArg>>,
}

// ---------------------------------------------------------------- items

#[derive(Clone, Debug)]
pub enum Item {
    Net(NetDecl),
    Var(VarDecl),
    /// non-ANSI port declaration in a module or subroutine body
    Port(PortDecl),
    Param(ParamDecl),
    Typedef(Typedef),
    Import(Vec<Import>, Loc),
    /// `export p::x;` / `export *::*;`: (package or "*", name or "*")
    Export(Vec<(String, String)>, Loc),
    Genvar(Vec<(String, Loc)>),
    ContAssign(ContAssign),
    /// `alias a = b = c;`
    NetAlias(Vec<Expr>, Loc),
    Process(Process),
    Subroutine(Box<Subroutine>),
    Dpi(Box<DpiDecl>),
    /// module / interface / program / checker / UDP instances (or a gate named by a user name)
    Instances(Instances),
    Gates(Gates),
    /// `generate ... endgenerate`
    Generate(Vec<Item>, Loc),
    GenFor(Box<GenFor>),
    GenIf(Box<GenIf>),
    GenCase(Box<GenCase>),
    /// `begin : name ... end` in a generate region
    GenBlock(GenBlock),
    Defparam(Vec<(Expr, Expr)>, Loc),
    Specify(Vec<SpecifyItem>, Loc),
    Modport(Vec<Modport>, Loc),
    Clocking(Box<ClockingDecl>),
    /// `default clocking name;` / `global clocking name;` referencing a block
    ClockingRef { global: bool, name: String, loc: Loc },
    /// `default disable iff (e);`
    DefaultDisable(Expr, Loc),
    /// a concurrent assertion item (`assert property ...` at module level)
    Assertion(Box<Assertion>),
    Property(Box<PropertyDecl>),
    Sequence(Box<SequenceDecl>),
    Let(Box<LetDecl>),
    Covergroup(Box<Covergroup>),
    Class(Box<ClassDecl>),
    /// `constraint C::name { ... }` outside its class
    Constraint(Box<ConstraintDecl>),
    Bind(Box<Bind>),
    /// a module/interface/program declared inside another
    Module(Box<ModuleDecl>),
    Timeunits { unit: Option<(f64, i32)>, precision: Option<(f64, i32)>, loc: Loc },
    /// `$error(...)` & co. as an elaboration-time item
    ElabTask { name: String, args: Vec<Arg>, loc: Loc },
    /// `nettype T name [with f];`
    Nettype { ty: DataType, name: String, resolve: Option<Name>, loc: Loc },
    Directive(Directive, Loc),
    /// a lone `;`
    Empty(Loc),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetTypeKw {
    Wire,
    Tri,
    Tri0,
    Tri1,
    Supply0,
    Supply1,
    Wand,
    Wor,
    Triand,
    Trior,
    Trireg,
    Uwire,
    Interconnect,
}

/// A drive strength `(strong0, weak1)` or charge strength `(small)`, as written.
#[derive(Clone, Debug, PartialEq)]
pub enum Strength {
    Drive(Kw, Option<Kw>),
    Charge(Kw),
}

#[derive(Clone, Debug)]
pub enum Delay {
    /// `#5`, `#d`, `#1ns`, `#(e)`
    Value(Expr),
    /// `#(rise, fall [, turn-off])` (each may be min:typ:max)
    List(Vec<Expr>),
}

#[derive(Clone, Debug)]
pub struct NetDecl {
    /// None: a user-defined nettype `my_net x;` (then `user` is its name)
    pub net: Option<NetTypeKw>,
    pub user: Option<Name>,
    pub strength: Option<Strength>,
    /// `vectored` / `scalared`
    pub vectored: Option<Kw>,
    pub ty: DataType,
    pub delay: Option<Delay>,
    pub decls: Vec<Declarator>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct VarDecl {
    pub const_: bool,
    /// `var` written
    pub var: bool,
    pub lifetime: Option<Lifetime>,
    pub ty: DataType,
    pub decls: Vec<Declarator>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Declarator {
    pub name: String,
    pub dims: Vec<Dim>,
    pub init: Option<Expr>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct PortDecl {
    pub dir: Dir,
    pub net: Option<NetTypeKw>,
    pub var: bool,
    /// an interface port declared in the body (`bus_if.mp p;` is not legal, kept for checkers)
    pub ty: DataType,
    pub decls: Vec<Declarator>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum ParamKind {
    /// `parameter [type] a = 1, b = 2`
    Value { ty: DataType, assigns: Vec<Declarator> },
    /// `parameter type T = int, U`
    Type { assigns: Vec<(String, Option<DataType>, Loc)> },
}

#[derive(Clone, Debug)]
pub struct ParamDecl {
    /// `parameter`, `localparam` or `specparam` (None in a parameter port list entry without
    /// a keyword)
    pub kw: Option<Kw>,
    pub kind: ParamKind,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum Typedef {
    Type { ty: DataType, name: String, dims: Vec<Dim>, loc: Loc },
    /// `typedef [enum|struct|union|class|interface class] name;`
    Forward { kind: Option<Kw>, interface_class: bool, name: String, loc: Loc },
    /// `typedef iface_inst.T name;`
    IfaceType { iface: String, ty: String, name: String, loc: Loc },
}

#[derive(Clone, Debug)]
pub struct Import {
    pub package: String,
    /// None: `*`
    pub name: Option<String>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct ContAssign {
    pub strength: Option<Strength>,
    pub delay: Option<Delay>,
    pub assigns: Vec<(Expr, Expr, Loc)>,
    pub loc: Loc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcKind {
    Initial,
    Always,
    AlwaysComb,
    AlwaysFf,
    AlwaysLatch,
    Final,
}

#[derive(Clone, Debug)]
pub struct Process {
    pub kind: ProcKind,
    pub body: Stmt,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Subroutine {
    pub is_task: bool,
    /// class method qualifiers as written: virtual, pure, extern, static, local, protected
    pub quals: Vec<Kw>,
    pub lifetime: Option<Lifetime>,
    /// return type (functions; `void`); None: implicit
    pub ret: Option<DataType>,
    /// possibly `Class::name` for an out-of-class body; `new` for constructors
    pub name: Name,
    /// ANSI port list (None: no parentheses)
    pub ports: Option<Vec<TfPort>>,
    /// declarations and statements of the body (None: a prototype)
    pub body: Option<(Vec<Item>, Vec<Stmt>)>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct TfPort {
    pub dir: Option<Dir>,
    pub var: bool,
    /// as written (inherits from the previous port when it has no type and no direction)
    pub ty: Option<DataType>,
    pub name: String,
    pub dims: Vec<Dim>,
    pub default: Option<Expr>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct DpiDecl {
    pub import: bool,
    /// "DPI-C" / "DPI"
    pub spec: String,
    /// `context` / `pure`
    pub property: Option<Kw>,
    pub c_name: Option<String>,
    /// import: the prototype; export: `function name` / `task name`
    pub proto: Subroutine,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum ParamArg {
    Positional(Expr),
    /// an empty position `#( , 4)`
    Empty,
    /// `.name(value)` (None: `.name()`)
    Named(String, Option<Expr>),
}

#[derive(Clone, Debug)]
pub enum PortConn {
    Positional(Option<Expr>),
    /// `.name(expr)` / `.name()` (Some(None)) / `.name` (None)
    Named(String, Option<Option<Expr>>),
    /// `.*`
    Wildcard,
}

#[derive(Clone, Debug)]
pub struct Instance {
    /// None: an unnamed instance
    pub name: Option<String>,
    pub dims: Vec<Dim>,
    pub conns: Vec<PortConn>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Instances {
    pub module: String,
    pub params: Option<Vec<ParamArg>>,
    pub insts: Vec<Instance>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct GateInst {
    pub name: Option<String>,
    pub dims: Vec<Dim>,
    pub terms: Vec<Expr>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Gates {
    pub kind: Kw,
    pub strength: Option<Strength>,
    pub delay: Option<Delay>,
    pub insts: Vec<GateInst>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct GenBlock {
    pub name: Option<String>,
    /// written with `begin ... end` (a block of a conditional without it that is just
    /// another conditional makes no scope of its own, IEEE 1800 27.5)
    pub begin: bool,
    pub items: Vec<Item>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct GenFor {
    /// `genvar` written in the header
    pub genvar: bool,
    pub var: String,
    pub init: Expr,
    pub cond: Expr,
    /// the step as written: `i = i + 1`, `i++`, `i += 2`
    pub step: Stmt,
    pub body: GenBlock,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct GenIf {
    pub cond: Expr,
    pub then_: GenBlock,
    pub else_: Option<GenBlock>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct GenCase {
    pub expr: Expr,
    /// labels (empty: default) and the body
    pub arms: Vec<(Vec<Expr>, GenBlock)>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Modport {
    pub name: String,
    pub ports: Vec<ModportPort>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum ModportPort {
    /// `input a, .b(expr)` (expr for an explicit port)
    Simple { dir: Dir, name: String, expr: Option<Expr> },
    /// `clocking cb`
    Clocking(String),
    /// `import f, task t(...)` / `export ...`
    Tf { import: bool, name: String, proto: Option<Box<Subroutine>> },
}

#[derive(Clone, Debug)]
pub struct UdpDecl {
    pub name: String,
    pub ports: Vec<String>,
    /// port declarations (ANSI ones also appear here)
    pub decls: Vec<Item>,
    /// `initial q = 1'b0;`
    pub init: Option<(String, Expr)>,
    /// the raw table rows: inputs, current state (sequential), output
    pub rows: Vec<(Vec<String>, Option<String>, String)>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct PackageDecl {
    pub name: String,
    pub lifetime: Option<Lifetime>,
    pub items: Vec<Item>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct ConfigDecl {
    pub name: String,
    /// `design lib.cell ...;`
    pub design: Vec<String>,
    /// the rules, kept as text
    pub rules: Vec<String>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Bind {
    /// the target module (or instance path) and optional instance list after `:`
    pub target: Expr,
    pub targets: Vec<Expr>,
    pub inst: Instances,
    pub loc: Loc,
}

// ---------------------------------------------------------------- specify

#[derive(Clone, Debug)]
pub enum SpecifyItem {
    Specparam(ParamDecl),
    Path(Box<SpecPath>),
    /// `$setup(data, posedge clk, 3, notifier);` & co.: the arguments as written
    TimingCheck { name: String, args: Vec<Option<TimingCheckArg>>, loc: Loc },
    /// `pulsestyle_onevent a;`, `showcancelled b;` ...
    Pulse { kw: Kw, names: Vec<String>, loc: Loc },
}

#[derive(Clone, Debug)]
pub struct TimingCheckArg {
    /// posedge / negedge / edge [01, 10]
    pub edge: Option<Edge>,
    pub expr: Expr,
    /// `&&& cond`
    pub cond: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct SpecPath {
    /// `if (c)` (Some(Some)), `ifnone` (Some(None))
    pub cond: Option<Option<Expr>>,
    pub edge: Option<Edge>,
    pub srcs: Vec<Expr>,
    /// `=>` (parallel) / `*>` (full), with polarity `+` / `-`
    pub full: bool,
    pub polarity: Option<char>,
    pub dsts: Vec<Expr>,
    /// edge-sensitive path: `(posedge clk => (q +: d))`: the data source
    pub data: Option<Expr>,
    pub delays: Delay,
    pub loc: Loc,
}

// ---------------------------------------------------------------- statements

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    /// `label: stmt`
    pub label: Option<String>,
    pub loc: Loc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    All,
    Any,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseKw {
    Case,
    Casez,
    Casex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseMode {
    Normal,
    Inside,
    Matches,
}

/// `unique`, `unique0`, `priority`
pub type Qualifier = Option<Kw>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Eq,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    AShl,
    AShr,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Null,
    /// `begin` / `fork`: declarations, then statements
    Block { fork: Option<Join>, name: Option<String>, decls: Vec<Item>, stmts: Vec<Stmt> },
    /// blocking (`op` Eq or compound) assignment
    Assign { lhs: Expr, op: AssignOp, timing: Option<Timing>, rhs: Expr },
    NonBlocking { lhs: Expr, timing: Option<Timing>, rhs: Expr },
    /// an expression statement: a subroutine call, `x++`, `void'(f())`, `obj.randomize() with {}`
    Expr(Expr),
    If { qual: Qualifier, cond: Expr, then_: Box<Stmt>, else_: Option<Box<Stmt>> },
    Case { qual: Qualifier, kw: CaseKw, expr: Expr, mode: CaseMode, items: Vec<CaseItem> },
    For { init: Vec<ForInit>, cond: Option<Expr>, step: Vec<Stmt>, body: Box<Stmt> },
    Foreach { array: Expr, vars: Vec<Option<String>>, body: Box<Stmt> },
    While { cond: Expr, body: Box<Stmt> },
    DoWhile { body: Box<Stmt>, cond: Expr },
    Repeat { count: Expr, body: Box<Stmt> },
    Forever(Box<Stmt>),
    Timed { timing: Timing, body: Box<Stmt> },
    Wait { cond: Expr, body: Box<Stmt> },
    WaitFork,
    WaitOrder { events: Vec<Expr>, action: Box<ActionBlock> },
    /// `-> e` / `->> [timing] e`
    Trigger { nonblocking: bool, timing: Option<Timing>, target: Expr },
    Disable(Expr),
    DisableFork,
    /// `assign` / `deassign` / `force` / `release` (rhs None for the latter two)
    ProcCont { kw: Kw, lhs: Expr, rhs: Option<Expr> },
    Return(Option<Expr>),
    Break,
    Continue,
    RandCase(Vec<(Expr, Stmt)>),
    RandSequence { start: Option<String>, prods: Vec<Production> },
    /// immediate, deferred or concurrent assertion statement
    Assertion(Box<Assertion>),
    /// `expect (property) action`
    Expect { prop: Prop, action: Box<ActionBlock> },
    /// clocking drive `cb.x <= ##2 v;` is a NonBlocking with timing Cycle
    /// a statement that is only a declaration-like item in a statement position (`let`)
    Item(Box<Item>),
}

#[derive(Clone, Debug)]
pub struct CaseItem {
    /// empty: `default` (and no pattern); ranges only with `inside`
    pub labels: Vec<ValueRange>,
    /// `case ... matches`: the pattern and its `&&& guard`
    pub pattern: Option<Pattern>,
    pub guard: Option<Expr>,
    pub body: Stmt,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum ForInit {
    /// `int i = 0` (the type given, or inherited from the previous one in the list)
    Decl { var: bool, ty: Option<DataType>, name: String, init: Expr, loc: Loc },
    Assign(Stmt),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Pos,
    Neg,
    /// `edge` (with optional descriptors in timing checks)
    Both,
}

#[derive(Clone, Debug)]
pub struct EventExpr {
    pub edge: Option<Edge>,
    pub expr: Expr,
    pub iff: Option<Expr>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum EventCtl {
    /// `@*`, `@(*)`
    Star,
    /// `@name`, `@a.b`, `@(list)` with `or` / `,`
    List(Vec<EventExpr>),
    /// `@(sequence_instance)` and other forms kept as one expression
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub enum Timing {
    Delay(Delay),
    Event(EventCtl),
    /// `##n`, `##[a:b]` (cycle delay)
    Cycle(Expr),
    /// intra-assignment `repeat (n) @(ev)`
    Repeat(Expr, EventCtl),
}

#[derive(Clone, Debug)]
pub struct ActionBlock {
    pub pass: Option<Stmt>,
    pub fail: Option<Stmt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssertKind {
    Assert,
    Assume,
    Cover,
    Restrict,
}

#[derive(Clone, Debug)]
pub enum AssertBody {
    /// immediate: `assert (e)`; deferred: `assert #0 (e)` / `assert final (e)`
    Immediate { deferred: Option<Kw>, expr: Expr },
    /// `assert property (p)`
    Property(Prop),
    /// `cover sequence (s)`
    Sequence(Prop),
}

#[derive(Clone, Debug)]
pub struct Assertion {
    pub label: Option<String>,
    pub kind: AssertKind,
    pub body: AssertBody,
    pub action: ActionBlock,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Production {
    pub ret: Option<DataType>,
    pub name: String,
    pub ports: Option<Vec<TfPort>>,
    /// alternatives separated by `|`: items and an optional `:= weight`
    pub rules: Vec<(Vec<ProdItem>, Option<Expr>)>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum ProdItem {
    /// a production name with arguments
    Call(String, Vec<Arg>),
    /// `{ code }`
    Code(Vec<Item>, Vec<Stmt>),
    If { cond: Expr, then_: Box<ProdItem>, else_: Option<Box<ProdItem>> },
    Repeat { count: Expr, item: Box<ProdItem> },
    Case { expr: Expr, items: Vec<(Vec<Expr>, ProdItem)> },
    /// `rand join [(e)] a b c`
    RandJoin { weight: Option<Expr>, items: Vec<ProdItem> },
}

// ---------------------------------------------------------------- expressions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// `->`
    LImpl,
    /// `<->`
    LEquiv,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeKind {
    /// `[a:b]`
    Colon,
    /// `[a+:w]`
    Plus,
    /// `[a-:w]`
    Minus,
}

#[derive(Clone, Debug)]
pub enum Arg {
    /// a positional argument (None: empty, `f(a, , b)`)
    Pos(Option<Expr>),
    /// `.name(e)` / `.name()`
    Named(String, Option<Expr>),
}

#[derive(Clone, Debug)]
pub enum ValueRange {
    Value(Expr),
    /// `[lo:hi]` (a `$` bound is Expr::Dollar); 1800-2023 tolerance forms are not parsed
    Range(Expr, Expr),
}

#[derive(Clone, Debug)]
pub struct DistItem {
    pub range: ValueRange,
    /// `:=` (false) / `:/` (true) and the weight; None: weight 1
    pub weight: Option<(bool, Expr)>,
}

#[derive(Clone, Debug)]
pub enum PatternItems {
    /// `'{a, b}`
    Positional(Vec<Expr>),
    /// `'{name: v, int: v, default: v, 3: v}`: keys as expressions (a type key is
    /// `Expr::Type`, `default` is `Expr::Default`)
    Keyed(Vec<(Expr, Expr)>),
    /// `'{n{a, b}}`
    Repl(Box<Expr>, Vec<Expr>),
}

#[derive(Clone, Debug)]
pub enum CastTarget {
    Type(Box<DataType>),
    /// `8'(e)`, `(W+1)'(e)`
    Size(Box<Expr>),
    Signed,
    Unsigned,
    Const,
}

/// A pattern in `case ... matches` / `if (e matches p)`.
#[derive(Clone, Debug)]
pub enum Pattern {
    /// `.name` (binds a variable)
    Var(String),
    /// `.*`
    Wild,
    Expr(Expr),
    Tagged(String, Option<Box<Pattern>>),
    /// `'{p, q}` / `'{name: p}`
    List(Vec<(Option<String>, Pattern)>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Num(Number),
    Real(f64),
    /// time literal: value, unit (power of ten of seconds)
    Time(f64, i32),
    Str(String),
    Null,
    This,
    Super,
    /// `$` (queue end, open range bound)
    Dollar,
    /// `1step`
    Step,
    /// `default` as a pattern key
    Default,
    Ident(String),
    /// `p::x`, `$unit::x`, `C#(T)::x`, `local::x`
    Scoped(Box<Name>),
    /// `$root`
    Root,
    /// `a.b`: a hierarchical step or a member
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Range(Box<Expr>, RangeKind, Box<Expr>, Box<Expr>),
    /// `f(args)`, `a.b.f(args)`, `p::f(args)`, `x.m() with (e)`, `arr.find(x) with (x > 1)`,
    /// a call without parentheses has `parens: false`
    Call { func: Box<Expr>, args: Vec<Arg>, parens: bool, with_: Option<Box<With>> },
    SysCall { name: String, args: Vec<Arg>, parens: bool },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `++x` / `x--` (prefix, op '+' or '-')
    IncDec { prefix: bool, inc: bool, e: Box<Expr> },
    /// an assignment in an expression `(a = b)`, `(a += 1)`
    Assign(AssignOp, Box<Expr>, Box<Expr>),
    Inside(Box<Expr>, Vec<ValueRange>),
    Dist(Box<Expr>, Vec<DistItem>),
    /// `e matches pattern` (in a condition), followed by `&&& guards`
    Matches(Box<Expr>, Box<Pattern>),
    /// `a &&& b` in a condition
    TripleAnd(Box<Expr>, Box<Expr>),
    Concat(Vec<Expr>),
    Repl(Box<Expr>, Vec<Expr>),
    /// `{<< [slice] {items}}`: left (`<<`) or right, the slice (expression or type), the items
    /// with optional `with [range]`
    Stream { left: bool, slice: Option<Box<Expr>>, items: Vec<(Expr, Option<Box<Expr>>)> },
    /// `'{...}` (ty None) or `T'{...}`
    Pattern { ty: Option<Box<DataType>>, items: PatternItems },
    Cast(CastTarget, Box<Expr>),
    /// min:typ:max
    MinTypMax(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `tagged Name [expr]`
    Tagged(String, Option<Box<Expr>>),
    /// `new`, `new(args)`, `new[n]`, `new[n](init)`, `new obj` (copy)
    New { size: Option<Box<Expr>>, args: Option<Vec<Arg>>, copy: Option<Box<Expr>> },
    /// `C::new`, `C::new(args)`: a constructor call of a named class type
    TypedNew { class: Box<Name>, args: Option<Vec<Arg>> },
    /// a data type where an expression may be one (`$bits(T)`, parameter values, `type(T)`)
    Type(Box<DataType>),
    /// a property/sequence expression in an expression position (assertion arguments)
    Prop(Box<Prop>),
}

/// The `with` clause of an array method or `randomize`.
#[derive(Clone, Debug)]
pub enum With {
    /// `with (expr)`
    Expr(Expr),
    /// `with { constraints }` (randomize), optional identifier list `with (a, b) {...}`
    Constraint(Option<Vec<String>>, Vec<ConstraintItem>),
}

impl Expr {
    pub fn new(kind: ExprKind, loc: Loc) -> Expr {
        Expr { kind, loc }
    }
}

// ---------------------------------------------------------------- assertions

/// A property or sequence expression (IEEE 1800 16.7-16.12; one tree for both).
#[derive(Clone, Debug)]
pub enum Prop {
    Expr(Expr),
    /// `(seq, v = e, w++)`: a sequence with match items
    MatchItems(Box<Prop>, Vec<Expr>),
    /// `a ##n b`, `##[1:3] b` (lhs None)
    Delay(Option<Box<Prop>>, CycleRange, Box<Prop>),
    /// `a[*n]`, `a[=n]`, `a[->n]`, `a[*]`, `a[+]`
    Repeat(Box<Prop>, RepeatKind, Option<CycleRange>),
    Binary(PropOp, Box<Prop>, Box<Prop>),
    Not(Box<Prop>),
    /// `strong(s)` / `weak(s)`
    Strength(bool, Box<Prop>),
    /// `nexttime`, `s_nexttime`, `always`, `s_always`, `eventually`, `s_eventually` with an
    /// optional count/range
    Unary(Kw, Option<CycleRange>, Box<Prop>),
    /// `first_match(s, items)`
    FirstMatch(Box<Prop>, Vec<Expr>),
    /// `accept_on (e) p` & co.
    Abort(Kw, Expr, Box<Prop>),
    If(Expr, Box<Prop>, Option<Box<Prop>>),
    Case(Expr, Vec<(Vec<Expr>, Prop)>),
    /// `@(ev) p`
    Clocked(EventCtl, Box<Prop>),
    /// `disable iff (e) p`
    DisableIff(Expr, Box<Prop>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropOp {
    And,
    Or,
    Intersect,
    Within,
    Throughout,
    /// `|->`
    ImplOverlap,
    /// `|=>`
    ImplNext,
    /// `#-#`
    FollowOverlap,
    /// `#=#`
    FollowNext,
    Implies,
    Iff,
    Until,
    SUntil,
    UntilWith,
    SUntilWith,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatKind {
    /// `[*]`
    Consecutive,
    /// `[=]`
    NonConsecutive,
    /// `[->]`
    Goto,
    /// `[+]` (one or more)
    Plus,
}

/// `n`, `[a:b]`, `[a:$]`, `[*]` (zero or more), `[+]`
#[derive(Clone, Debug)]
pub enum CycleRange {
    One(Expr),
    Range(Expr, Expr),
}

#[derive(Clone, Debug)]
pub struct PropertyDecl {
    pub name: String,
    pub ports: Vec<TfPort>,
    pub decls: Vec<Item>,
    pub prop: Prop,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct SequenceDecl {
    pub name: String,
    pub ports: Vec<TfPort>,
    pub decls: Vec<Item>,
    pub seq: Prop,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct LetDecl {
    pub name: String,
    pub ports: Vec<TfPort>,
    pub expr: Expr,
    pub loc: Loc,
}

// ---------------------------------------------------------------- clocking

#[derive(Clone, Debug)]
pub struct ClockingDecl {
    /// `default` / `global`
    pub kind: Option<Kw>,
    pub name: Option<String>,
    pub event: EventCtl,
    pub items: Vec<ClockingItem>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Skew {
    pub edge: Option<Edge>,
    pub delay: Option<Expr>,
}

#[derive(Clone, Debug)]
pub enum ClockingItem {
    /// `default input #1 output #2;`
    Default { input: Option<Skew>, output: Option<Skew> },
    /// `input #1 a, b = top.x;` (dir Input / Output / Inout; skews for input and output)
    Signals { dir: Dir, input: Option<Skew>, output: Option<Skew>, names: Vec<(String, Option<Expr>)> },
    Item(Item),
}

// ---------------------------------------------------------------- classes

#[derive(Clone, Debug)]
pub struct ClassDecl {
    pub virtual_: bool,
    pub interface: bool,
    pub lifetime: Option<Lifetime>,
    pub name: String,
    pub params: Option<Vec<ParamDecl>>,
    /// `extends Base #(..) (args)`
    pub extends: Option<(Name, Option<Vec<Arg>>)>,
    pub implements: Vec<Name>,
    pub items: Vec<ClassItem>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum ClassItem {
    /// a property with its qualifiers (rand, randc, static, local, protected, const)
    Property { quals: Vec<Kw>, decl: Item },
    Method(Box<Subroutine>),
    Constraint(Box<ConstraintDecl>),
    Item(Item),
}

#[derive(Clone, Debug)]
pub struct ConstraintDecl {
    pub static_: bool,
    /// `extern` / `pure` prototype qualifiers
    pub quals: Vec<Kw>,
    /// possibly `C::name`
    pub name: Name,
    /// None: a prototype
    pub items: Option<Vec<ConstraintItem>>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum ConstraintItem {
    Expr { soft: bool, expr: Expr },
    /// `e -> constraint_set`
    Implies(Expr, Vec<ConstraintItem>),
    If(Expr, Vec<ConstraintItem>, Option<Vec<ConstraintItem>>),
    Foreach(Expr, Vec<Option<String>>, Vec<ConstraintItem>),
    Solve(Vec<Expr>, Vec<Expr>),
    Unique(Vec<ValueRange>),
    DisableSoft(Expr),
    /// a nested `{ ... }`
    Block(Vec<ConstraintItem>),
}

// ---------------------------------------------------------------- coverage

#[derive(Clone, Debug)]
pub struct Covergroup {
    pub name: String,
    pub ports: Vec<TfPort>,
    /// `@(ev)`, `@@(begin f)`, `with function sample(args)`
    pub event: Option<CoverEvent>,
    pub items: Vec<CoverItem>,
    /// `extends base` (1800-2023, accepted)
    pub extends: Option<String>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum CoverEvent {
    Event(EventCtl),
    Sample(Vec<TfPort>),
    /// `@@( begin f or end g )`: kept as an expression
    Block(Expr),
}

#[derive(Clone, Debug)]
pub enum CoverItem {
    /// `option.x = v;` / `type_option.x = v;`
    Option { type_option: bool, name: String, value: Expr },
    Coverpoint(Box<Coverpoint>),
    Cross(Box<Cross>),
}

#[derive(Clone, Debug)]
pub struct Coverpoint {
    pub ty: Option<DataType>,
    pub label: Option<String>,
    pub expr: Expr,
    pub iff: Option<Expr>,
    pub bins: Vec<BinsItem>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub struct Cross {
    pub label: Option<String>,
    pub items: Vec<Expr>,
    pub iff: Option<Expr>,
    pub bins: Vec<BinsItem>,
    pub loc: Loc,
}

#[derive(Clone, Debug)]
pub enum BinsValues {
    Ranges(Vec<ValueRange>),
    /// `with (e)` / a set expression
    Expr(Expr),
    Default,
    DefaultSequence,
    /// `(1 => 2 => 3[*2]), (...)`: transitions, each a list of (ranges, repetition)
    Trans(Vec<Vec<(Vec<ValueRange>, Option<(RepeatKind, CycleRange)>)>>),
    /// cross bins: `binsof(a) intersect {...} && ...`
    Select(Expr),
}

#[derive(Clone, Debug)]
pub struct BinsItem {
    /// `bins` / `illegal_bins` / `ignore_bins`, or an option (`option.x = v` in a coverpoint)
    pub kind: Option<Kw>,
    pub wildcard: bool,
    pub name: String,
    /// `name[]` (Some(None)) / `name[n]`
    pub array: Option<Option<Expr>>,
    pub values: BinsValues,
    pub with_: Option<Expr>,
    pub iff: Option<Expr>,
    /// an option instead of a bin: `option.weight = 2;`
    pub option: Option<(String, Expr)>,
    pub loc: Loc,
}
