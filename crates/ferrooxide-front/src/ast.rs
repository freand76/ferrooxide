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
    /// names that are SystemVerilog keywords (legal Verilog-2005 names; for the lint check)
    pub keyword_names: Vec<(String, Line)>,
    /// an `interface ... endinterface`
    pub interface: bool,
    /// interface ports (`bus_if.master b`): (port name, interface name)
    pub iface_ports: Vec<(String, String)>,
    /// modport names of an interface
    pub modports: Vec<String>,
    /// a package (or the file scope, `$unit`) holding variables: elaborated once, not a
    /// top-level module
    pub package: bool,
    /// packages (and `$unit`) whose variables the module sees after its own names
    pub imports: Vec<String>,
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
    Bit,
}

pub type Range = (Expr, Expr);

#[derive(Debug, Clone)]
pub struct Declarator {
    pub name: String,
    pub dims: Vec<Range>,
    pub init: Option<Expr>,
    pub line: Line,
}

#[derive(Debug, Clone)]
pub struct Decl {
    pub dir: Option<Dir>,
    pub kind: NetKind,
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
    pub signed: bool,
    pub range: Option<Range>,
    /// further packed dimensions (`reg [3:0][7:0] x`: range [3:0], packed [[7:0]])
    pub packed: Vec<Range>,
    /// declared with a type parameter: its type (per instance) replaces kind, sign and ranges
    pub type_param: Option<String>,
    pub names: Vec<Declarator>,
    pub line: Line,
}

#[derive(Debug, Clone)]
pub struct ParamDecl {
    pub local: bool,
    pub signed: bool,
    pub range: Option<Range>,
    /// `integer` type
    pub integer: bool,
    pub real: bool,
    pub name: String,
    pub value: Expr,
    pub line: Line,
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
    /// return type (functions only)
    pub signed: bool,
    pub range: Option<Range>,
    pub ret_kind: NetKind,
    /// ports in order + local declarations
    pub decls: Vec<Decl>,
    pub body: Stmt,
    pub line: Line,
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
    TypeParam { local: bool, name: String, default: Option<std::rc::Rc<crate::svtypes::SvType>>, line: Line },
    /// a `specify` path delay: `(a, b *> q) = (rise, fall);` (conditions and edges ignored)
    SpecPath { srcs: Vec<String>, dsts: Vec<String>, delay: Expr, line: Line },
    /// `defparam a.b.P = value;` (the target path includes the parameter name)
    Defparam { target: Vec<String>, value: Expr, line: Line },
    /// `strength`: drive strength (strength0, strength1), levels 0 (highz) .. 7 (supply)
    ContAssign { delay: Option<Expr>, lhs: Expr, rhs: Expr, strength: Option<(u8, u8)>, line: Line },
    Process { kind: ProcKind, body: Stmt, line: Line },
    Instances { module: String, params: Vec<Conn>, insts: Vec<Instance>, line: Line },
    Subroutine(SubroutineDecl),
    GenFor { init_var: String, init: Expr, cond: Expr, step_var: String, step: Expr, name: Option<String>, items: Vec<Item>, line: Line },
    GenIf { cond: Expr, then_items: Vec<Item>, then_name: Option<String>, else_items: Vec<Item>, else_name: Option<String>, line: Line },
    GenCase { expr: Expr, arms: Vec<(Vec<Expr>, Option<String>, Vec<Item>)>, line: Line },
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
    Block { name: Option<String>, decls: Vec<Decl>, stmts: Vec<Stmt>, fork: Option<Join> },
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
    Disable { name: String, line: Line },
    /// `force lhs = rhs` (`rhs` None: `release lhs`)
    Force { lhs: Expr, rhs: Option<Expr>, line: Line },
    Trigger { name: String, line: Line },
    Return { value: Option<Expr>, line: Line },
    Break { line: Line },
    Continue { line: Line },
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

#[derive(Debug, Clone)]
pub enum Expr {
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
    Type(std::rc::Rc<crate::svtypes::SvType>),
}
