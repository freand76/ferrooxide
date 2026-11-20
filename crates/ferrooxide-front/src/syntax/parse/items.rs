//! Module, interface, package and block items; declarations (IEEE 1800 A.1.4-A.2, A.4).

use super::types::is_type_kw;
use super::Parser;
use crate::syntax::kw::Kw;
use crate::syntax::token::T;
use crate::syntax::tree::*;
use crate::Result;

pub(crate) fn net_kw(k: Kw) -> Option<NetTypeKw> {
    Some(match k {
        Kw::Wire => NetTypeKw::Wire,
        Kw::Tri => NetTypeKw::Tri,
        Kw::Tri0 => NetTypeKw::Tri0,
        Kw::Tri1 => NetTypeKw::Tri1,
        Kw::Supply0 => NetTypeKw::Supply0,
        Kw::Supply1 => NetTypeKw::Supply1,
        Kw::Wand => NetTypeKw::Wand,
        Kw::Wor => NetTypeKw::Wor,
        Kw::Triand => NetTypeKw::Triand,
        Kw::Trior => NetTypeKw::Trior,
        Kw::Trireg => NetTypeKw::Trireg,
        Kw::Uwire => NetTypeKw::Uwire,
        Kw::Interconnect => NetTypeKw::Interconnect,
        _ => return None,
    })
}

pub(crate) fn is_gate_kw(k: Kw) -> bool {
    matches!(
        k,
        Kw::And
            | Kw::Or
            | Kw::Nand
            | Kw::Nor
            | Kw::Xor
            | Kw::Xnor
            | Kw::Not
            | Kw::Buf
            | Kw::Bufif0
            | Kw::Bufif1
            | Kw::Notif0
            | Kw::Notif1
            | Kw::Nmos
            | Kw::Pmos
            | Kw::Rnmos
            | Kw::Rpmos
            | Kw::Cmos
            | Kw::Rcmos
            | Kw::Pullup
            | Kw::Pulldown
            | Kw::Tran
            | Kw::Rtran
            | Kw::Tranif0
            | Kw::Tranif1
            | Kw::Rtranif0
            | Kw::Rtranif1
    )
}

fn is_strength_kw(k: Kw) -> bool {
    matches!(
        k,
        Kw::Supply0
            | Kw::Supply1
            | Kw::Strong0
            | Kw::Strong1
            | Kw::Pull0
            | Kw::Pull1
            | Kw::Weak0
            | Kw::Weak1
            | Kw::Highz0
            | Kw::Highz1
            | Kw::Small
            | Kw::Medium
            | Kw::Large
    )
}

pub(crate) fn dir_kw(k: Kw) -> Option<Dir> {
    Some(match k {
        Kw::Input => Dir::Input,
        Kw::Output => Dir::Output,
        Kw::Inout => Dir::Inout,
        Kw::Ref => Dir::Ref,
        _ => return None,
    })
}

impl Parser {
    /// One module/interface/program/package/generate item (several items may be added).
    pub(crate) fn module_item(&mut self, out: &mut Vec<Item>) -> Result<()> {
        let loc = self.loc();
        // `label: assert property ...` / `label: begin ... end`
        if self.is_ident() && self.is_p_at(1, ":") && !self.soft_name_here() {
            let label = self.ident()?;
            self.bump();
            if matches!(self.kw(), Some(Kw::Assert | Kw::Assume | Kw::Cover | Kw::Restrict)) {
                let a = self.assertion(Some(label))?;
                out.push(Item::Assertion(Box::new(a)));
                return Ok(());
            }
            if self.is_kw(Kw::Begin) {
                self.block_label = Some(label.clone());
                let mut b = self.gen_block()?;
                if b.name.is_none() {
                    b.name = Some(label);
                }
                out.push(Item::GenBlock(b));
                return Ok(());
            }
            // a labeled generate construct
            let mut v = Vec::new();
            self.module_item(&mut v)?;
            out.push(Item::GenBlock(GenBlock { name: Some(label), begin: true, items: v, loc }));
            return Ok(());
        }
        if self.soft_name_here() {
            return self.err(format!("unexpected {} in module", self.describe()));
        }
        match self.peek().clone() {
            T::Punct(";") => {
                self.bump();
                out.push(Item::Empty(loc));
            }
            T::Directive(_) => {
                let d = self.directive()?;
                out.push(Item::Directive(d, loc));
            }
            T::SysIdent(name) => {
                self.bump();
                let args = if self.is_p("(") { self.args()? } else { vec![] };
                self.expect_p(";")?;
                out.push(Item::ElabTask { name, args, loc });
            }
            T::Kw(k) => self.kw_item(k, out)?,
            T::Ident(_) => self.ident_item(out)?,
            _ => return self.err(format!("unexpected {} in module", self.describe())),
        }
        Ok(())
    }

    fn kw_item(&mut self, k: Kw, out: &mut Vec<Item>) -> Result<()> {
        let loc = self.loc();
        match k {
            Kw::Timeunit | Kw::Timeprecision if self.in_generate > 0 => {
                return self.err(format!("{} is not allowed in a generate block", k.as_str()))
            }
            Kw::Timeunit | Kw::Timeprecision => out.push(self.timeunits()?),
            Kw::Specparam if self.in_generate > 0 => return self.err("a specparam is not allowed in a generate block"),
            Kw::Parameter | Kw::Localparam | Kw::Specparam => {
                let p = self.param_decl(true)?;
                self.expect_p(";")?;
                out.push(Item::Param(p));
            }
            Kw::Typedef => out.push(self.typedef()?),
            Kw::Import if matches!(self.peek_at(1), T::Str(_)) => out.push(Item::Dpi(Box::new(self.dpi()?))),
            Kw::Export if matches!(self.peek_at(1), T::Str(_)) => out.push(Item::Dpi(Box::new(self.dpi()?))),
            Kw::Import => out.push(self.import_decl()?),
            Kw::Export => {
                self.bump();
                let mut v = Vec::new();
                loop {
                    let p = if self.eat_p("*") { "*".to_string() } else { self.ident()? };
                    self.expect_p("::")?;
                    let n = if self.eat_p("*") { "*".to_string() } else { self.ident()? };
                    v.push((p, n));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                out.push(Item::Export(v, loc));
            }
            Kw::Modport => out.push(self.modports()?),
            Kw::Assign => out.push(self.cont_assign()?),
            Kw::Alias => {
                self.bump();
                let mut v = vec![self.postfix_expr()?];
                while self.eat_p("=") {
                    v.push(self.postfix_expr()?);
                }
                self.expect_p(";")?;
                out.push(Item::NetAlias(v, loc));
            }
            Kw::Initial | Kw::Always | Kw::AlwaysComb | Kw::AlwaysFf | Kw::AlwaysLatch | Kw::Final => {
                self.bump();
                let kind = match k {
                    Kw::Initial => ProcKind::Initial,
                    Kw::Always => ProcKind::Always,
                    Kw::AlwaysComb => ProcKind::AlwaysComb,
                    Kw::AlwaysFf => ProcKind::AlwaysFf,
                    Kw::AlwaysLatch => ProcKind::AlwaysLatch,
                    _ => ProcKind::Final,
                };
                let body = self.stmt()?;
                out.push(Item::Process(Process { kind, body, loc }));
            }
            Kw::Function | Kw::Task => out.push(Item::Subroutine(Box::new(self.subroutine(vec![])?))),
            Kw::Generate => {
                self.bump();
                let mut v = Vec::new();
                self.in_generate += 1;
                while !self.eat_kw(Kw::Endgenerate) {
                    if self.at_eof() {
                        return self.err("unexpected end of file in generate region");
                    }
                    self.module_item(&mut v)?;
                }
                self.in_generate -= 1;
                out.push(Item::Generate(v, loc));
            }
            Kw::For => out.push(self.gen_for()?),
            Kw::If => out.push(self.gen_if()?),
            Kw::Case => out.push(self.gen_case()?),
            Kw::Begin => out.push(Item::GenBlock(self.gen_block()?)),
            Kw::Defparam => {
                self.bump();
                let mut v = Vec::new();
                loop {
                    let target = self.postfix_expr()?;
                    self.expect_p("=")?;
                    let value = self.mintypmax()?;
                    v.push((target, value));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                out.push(Item::Defparam(v, loc));
            }
            Kw::Specify if self.in_generate > 0 => return self.err("a specify block is not allowed in a generate block"),
            Kw::Specify => out.push(self.specify()?),
            Kw::Genvar => {
                self.bump();
                let mut v = Vec::new();
                loop {
                    let l = self.loc();
                    v.push((self.ident()?, l));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                out.push(Item::Genvar(v));
            }
            Kw::Clocking => out.push(Item::Clocking(Box::new(self.clocking(None)?))),
            Kw::Default if self.is_kw_at(1, Kw::Clocking) => {
                self.bump();
                if self.is_ident_at(1) && self.is_p_at(2, ";") {
                    self.bump();
                    let name = self.ident()?;
                    self.expect_p(";")?;
                    out.push(Item::ClockingRef { global: false, name, loc });
                } else {
                    out.push(Item::Clocking(Box::new(self.clocking(Some(Kw::Default))?)));
                }
            }
            Kw::Global if self.is_kw_at(1, Kw::Clocking) => {
                self.bump();
                if self.is_ident_at(1) && self.is_p_at(2, ";") {
                    self.bump();
                    let name = self.ident()?;
                    self.expect_p(";")?;
                    out.push(Item::ClockingRef { global: true, name, loc });
                } else {
                    out.push(Item::Clocking(Box::new(self.clocking(Some(Kw::Global))?)));
                }
            }
            Kw::Default if self.is_kw_at(1, Kw::Disable) => {
                self.bump();
                self.bump();
                self.expect_kw(Kw::Iff)?;
                let e = self.expr()?;
                self.expect_p(";")?;
                out.push(Item::DefaultDisable(e, loc));
            }
            Kw::Assert | Kw::Assume | Kw::Cover | Kw::Restrict => out.push(Item::Assertion(Box::new(self.assertion(None)?))),
            Kw::Property => out.push(Item::Property(Box::new(self.property_decl()?))),
            Kw::Sequence => out.push(Item::Sequence(Box::new(self.sequence_decl()?))),
            Kw::Let => out.push(self.let_decl()?),
            Kw::Covergroup => out.push(Item::Covergroup(Box::new(self.covergroup()?))),
            Kw::Class => out.push(Item::Class(Box::new(self.class_decl()?))),
            Kw::Virtual if self.is_kw_at(1, Kw::Class) => out.push(Item::Class(Box::new(self.class_decl()?))),
            Kw::Interface if self.is_kw_at(1, Kw::Class) => out.push(Item::Class(Box::new(self.class_decl()?))),
            Kw::Constraint | Kw::Static if k == Kw::Constraint || self.is_kw_at(1, Kw::Constraint) => {
                out.push(Item::Constraint(Box::new(self.constraint_decl(vec![])?)))
            }
            Kw::Module | Kw::Macromodule => out.push(Item::Module(Box::new(self.module_decl(false)?))),
            // `interface u (...);` in Verilog code: an instance of a module named `interface`
            Kw::Interface | Kw::Program | Kw::Checker if self.nested_decl_ends(k) => {
                out.push(Item::Module(Box::new(self.module_decl(false)?)))
            }
            Kw::Extern if matches!(self.peek_at(1), T::Kw(Kw::Module | Kw::Macromodule | Kw::Interface | Kw::Program)) => {
                self.bump();
                out.push(Item::Module(Box::new(self.module_decl(true)?)))
            }
            Kw::Bind => out.push(Item::Bind(Box::new(self.bind()?))),
            Kw::Nettype => {
                self.bump();
                let Some(ty) = self.data_type()? else {
                    return self.err(format!("expected a data type, found {}", self.describe()));
                };
                let name = self.ident()?;
                let resolve = if self.eat_kw(Kw::With) { Some(self.scoped_name()?) } else { None };
                self.expect_p(";")?;
                self.add_type(&name);
                out.push(Item::Nettype { ty, name, resolve, loc });
            }
            k if is_gate_kw(k) => out.push(self.gates()?),
            _ if self.is_decl_start() => out.push(self.data_decl()?),
            k if !k.is_hard() => self.ident_item(out)?,
            _ => return self.err(format!("unexpected '{}' in module", k.as_str())),
        }
        Ok(())
    }

    /// A nested interface / program / checker declaration: its own end keyword comes before
    /// the end of the enclosing one.
    fn nested_decl_ends(&self, k: Kw) -> bool {
        let end = match k {
            Kw::Interface => Kw::Endinterface,
            Kw::Program => Kw::Endprogram,
            _ => Kw::Endchecker,
        };
        let mut i = 1;
        loop {
            match self.peek_at(i) {
                T::Kw(e) if *e == end => return true,
                T::Kw(Kw::Endmodule | Kw::Endinterface | Kw::Endprogram | Kw::Endchecker | Kw::Endpackage) | T::Eof => return false,
                _ => i += 1,
            }
        }
    }

    /// An item starting with a name: a declaration with a user type, or an instantiation.
    fn ident_item(&mut self, out: &mut Vec<Item>) -> Result<()> {
        // `process p();`: an instance of a module named like a type
        let instance = self.is_ident_at(1) && self.paren_after_dims(2);
        if self.is_type_start() && !instance {
            out.push(self.data_decl()?);
            return Ok(());
        }
        // `foo bar;` / `foo bar = x;` / `foo bar, baz;`: a declaration with a type the parser
        // does not know (a user-defined nettype, a type from elsewhere)
        if self.is_ident_at(1) && matches!(self.peek_at(2), T::Punct(";" | "=" | ",")) {
            out.push(self.data_decl_named()?);
            return Ok(());
        }
        // `pkg::T x;` with a type the parser does not know about
        if self.is_p_at(1, "::") {
            out.push(self.data_decl_named()?);
            return Ok(());
        }
        out.push(self.instances()?);
        Ok(())
    }

    /// From token `k` on: `[...]` groups, then `(`.
    fn paren_after_dims(&self, mut k: usize) -> bool {
        let mut depth = 0;
        loop {
            match self.peek_at(k) {
                T::Punct("[") => depth += 1,
                T::Punct("]") => depth -= 1,
                T::Punct("(") if depth == 0 => return true,
                T::Eof => return false,
                _ if depth == 0 => return false,
                _ => {}
            }
            k += 1;
        }
    }

    /// A declaration whose type is a name (known or not).
    fn data_decl_named(&mut self) -> Result<Item> {
        let loc = self.loc();
        let name = self.type_name()?;
        let signed = self.signing();
        let packed = self.packed_dims()?;
        let ty = DataType { kind: TypeKind::Named(Box::new(name)), signed, packed, loc };
        let decls = self.declarators()?;
        Ok(Item::Var(VarDecl { const_: false, var: false, lifetime: None, ty, decls, loc }))
    }

    /// A declaration (port, net or variable) starts here.
    pub(crate) fn is_decl_start(&self) -> bool {
        if self.soft_name_here() {
            return false;
        }
        match self.peek() {
            T::Kw(k) => {
                dir_kw(*k).is_some()
                    || net_kw(*k).is_some()
                    || matches!(k, Kw::Var | Kw::Const)
                    || (matches!(k, Kw::Static | Kw::Automatic) && (self.is_type_start_at(1) || self.is_kw_at(1, Kw::Var) || self.is_kw_at(1, Kw::Const)))
                    || (*k == Kw::Virtual && !self.is_kw_at(1, Kw::Class))
                    || (is_type_kw(*k) && !self.is_p_at(1, "'"))
                    || (*k == Kw::Type && self.is_p_at(1, "("))
            }
            _ => self.is_type_start(),
        }
    }

    /// A declaration in a block, function or task body starts here.
    pub(crate) fn is_block_decl_start(&self) -> bool {
        if self.soft_name_here() {
            return false;
        }
        match self.peek() {
            T::Kw(Kw::Typedef | Kw::Parameter | Kw::Localparam | Kw::Let | Kw::Import) => true,
            T::Kw(Kw::Genvar) => true,
            T::Kw(Kw::Static | Kw::Automatic) => true,
            _ => {
                if self.is_decl_start() {
                    return true;
                }
                // `T x;` with an unknown type name / `p::T x`
                matches!(self.peek(), T::Ident(_)) && self.is_type_start()
            }
        }
    }

    /// One declaration item in a block (variables, typedefs, parameters, ...).
    pub(crate) fn block_decl(&mut self, out: &mut Vec<Item>) -> Result<()> {
        let loc = self.loc();
        match self.kw() {
            Some(Kw::Typedef) => out.push(self.typedef()?),
            Some(Kw::Parameter | Kw::Localparam) => {
                let p = self.param_decl(true)?;
                self.expect_p(";")?;
                out.push(Item::Param(p));
            }
            Some(Kw::Let) => out.push(self.let_decl()?),
            Some(Kw::Import) => out.push(self.import_decl()?),
            Some(Kw::Genvar) => {
                self.bump();
                let mut v = Vec::new();
                loop {
                    let l = self.loc();
                    v.push((self.ident()?, l));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                out.push(Item::Genvar(v));
            }
            _ => {
                let _ = loc;
                out.push(self.data_decl()?)
            }
        }
        Ok(())
    }

    /// A port, net or variable declaration with its `;`.
    pub(crate) fn data_decl(&mut self) -> Result<Item> {
        let loc = self.loc();
        // port declaration
        let dir = if self.is_kw(Kw::Const) && self.is_kw_at(1, Kw::Ref) {
            self.bump();
            self.bump();
            Some(Dir::ConstRef)
        } else {
            self.kw().and_then(dir_kw).inspect(|_| {
                self.bump();
            })
        };
        if let Some(dir) = dir {
            let net = self.kw().and_then(net_kw).inspect(|_| {
                self.bump();
            });
            let var = !self.soft_name_here() && self.eat_kw(Kw::Var);
            let ty = self.decl_type()?;
            let decls = self.declarators()?;
            return Ok(Item::Port(PortDecl { dir, net, var, ty, decls, loc }));
        }
        // net declaration
        if let Some(net) = self.kw().and_then(net_kw) {
            self.bump();
            let strength = self.strength()?;
            let vectored = if self.is_kw(Kw::Vectored) || self.is_kw(Kw::Scalared) { self.kw().inspect(|_| { self.bump(); }) } else { None };
            let ty = self.decl_type()?;
            let delay = if self.eat_p("#") { Some(self.delay_value()?) } else { None };
            let decls = self.declarators()?;
            return Ok(Item::Net(NetDecl { net: Some(net), user: None, strength, vectored, ty, delay, decls, loc }));
        }
        // variable declaration
        let const_ = self.eat_kw(Kw::Const);
        let var = !self.soft_name_here() && self.eat_kw(Kw::Var);
        let lifetime = if self.eat_kw(Kw::Static) {
            Some(Lifetime::Static)
        } else if self.eat_kw(Kw::Automatic) {
            Some(Lifetime::Automatic)
        } else {
            None
        };
        let var = var || (!self.soft_name_here() && self.eat_kw(Kw::Var));
        let ty = self.decl_type()?;
        let decls = self.declarators()?;
        Ok(Item::Var(VarDecl { const_, var, lifetime, ty, decls, loc }))
    }

    /// The type of a declaration: a data type, or implicit; a soft keyword followed by what
    /// can only follow a name is the name (`reg logic;`).
    pub(crate) fn decl_type(&mut self) -> Result<DataType> {
        // `T x` with a type name the parser does not know (from a base class, a package of
        // another file, a nettype)
        if self.is_plain_ident_at(0) && !self.is_type_start() && (self.is_ident_at(1) || (self.is_p_at(1, "::") && self.is_ident_at(2))) {
            let loc = self.loc();
            let name = self.type_name()?;
            let signed = self.signing();
            let packed = self.packed_dims()?;
            return Ok(DataType { kind: TypeKind::Named(Box::new(name)), signed, packed, loc });
        }
        if self.soft_name_here() {
            let loc = self.loc();
            return Ok(DataType { kind: TypeKind::Implicit, signed: None, packed: vec![], loc });
        }
        let mut t = self.data_type_or_implicit()?;
        // `wire logic` / `wire real`: the data type after a net type, possibly after `logic`
        if matches!(t.kind, TypeKind::Int(IntKw::Logic | IntKw::Reg)) && t.signed.is_none() && t.packed.is_empty() && !self.soft_name_here() {
            if let Some(more) = self.data_type()? {
                // `wire logic real` is not legal; keep the second type
                t = more;
            }
        }
        Ok(t)
    }

    /// `(strong0, weak1)` / `(small)` / `(supply0)` after a net type or gate.
    pub(crate) fn strength(&mut self) -> Result<Option<Strength>> {
        if !(self.is_p("(") && matches!(self.peek_at(1), T::Kw(k) if is_strength_kw(*k))) {
            return Ok(None);
        }
        self.bump();
        let T::Kw(a) = self.bump() else { unreachable!() };
        let s = if matches!(a, Kw::Small | Kw::Medium | Kw::Large) {
            Strength::Charge(a)
        } else if self.eat_p(",") {
            match self.bump() {
                T::Kw(b) if is_strength_kw(b) => Strength::Drive(a, Some(b)),
                _ => return self.err("expected a strength"),
            }
        } else {
            Strength::Drive(a, None)
        };
        self.expect_p(")")?;
        Ok(Some(s))
    }

    /// `parameter ...` / `localparam ...` / `specparam ...` (without the `;`).
    pub(crate) fn param_decl(&mut self, _items: bool) -> Result<ParamDecl> {
        let loc = self.loc();
        let kw = self.kw();
        self.bump();
        if self.eat_kw(Kw::Type) {
            let assigns = self.type_param_assigns()?;
            return Ok(ParamDecl { kw, kind: ParamKind::Type { assigns }, loc });
        }
        let ty = self.param_value_type()?;
        let assigns = self.param_assigns()?;
        Ok(ParamDecl { kw, kind: ParamKind::Value { ty, assigns }, loc })
    }

    /// The type of a value parameter: a data type or implicit. A name followed by `=` is the
    /// parameter, not a type.
    fn param_value_type(&mut self) -> Result<DataType> {
        if self.is_ident() && (self.is_p_at(1, "=") || self.is_p_at(1, ",") || self.is_p_at(1, ")") || self.is_p_at(1, ";") || (self.is_p_at(1, "[") && !self.is_type_start())) {
            let loc = self.loc();
            return Ok(DataType { kind: TypeKind::Implicit, signed: None, packed: vec![], loc });
        }
        self.data_type_or_implicit()
    }

    /// `T = int, U` after `parameter type` (the names become type names).
    fn type_param_assigns(&mut self) -> Result<Vec<(String, Option<DataType>, Loc)>> {
        let mut v = Vec::new();
        loop {
            let loc = self.loc();
            let name = self.ident()?;
            let default = if self.eat_p("=") {
                match self.data_type()? {
                    Some(t) => Some(t),
                    None if self.is_ident() => {
                        let l = self.loc();
                        let n = self.type_name()?;
                        let signed = self.signing();
                        let packed = self.packed_dims()?;
                        Some(DataType { kind: TypeKind::Named(Box::new(n)), signed, packed, loc: l })
                    }
                    None => return self.err(format!("expected a data type, found {}", self.describe())),
                }
            } else {
                None
            };
            self.add_type(&name);
            v.push((name, default, loc));
            // another type parameter of the same list (not the next parameter port)
            if self.is_p(",") && self.is_ident_at(1) && !self.is_type_start_at(1) && matches!(self.peek_at(2), T::Punct("=" | "," | ")" | ";")) {
                self.bump();
                continue;
            }
            break;
        }
        Ok(v)
    }

    /// `a = 1, b [3:0] = 2` (stops before a `,` that starts another kind of parameter)
    fn param_assigns(&mut self) -> Result<Vec<Declarator>> {
        let mut v = Vec::new();
        loop {
            let loc = self.loc();
            let name = self.ident()?;
            let dims = self.unpacked_dims()?;
            let init = if self.eat_p("=") { Some(self.param_value()?) } else { None };
            v.push(Declarator { name, dims, init, loc });
            if self.is_p(",") && self.is_ident_at(1) && !self.is_type_start_at(1) && matches!(self.peek_at(2), T::Punct("=" | "," | ")" | ";" | "[")) && !matches!(self.peek_at(1), T::Kw(Kw::Parameter | Kw::Localparam | Kw::Type)) {
                self.bump();
                continue;
            }
            break;
        }
        Ok(v)
    }

    /// A parameter value: an expression (min:typ:max allowed) or a data type.
    fn param_value(&mut self) -> Result<Expr> {
        if self.is_type_start() && !self.is_p_at(1, "'") {
            let loc = self.loc();
            if let Some(t) = self.attempt(|p| {
                let t = p.data_type()?.ok_or_else(|| crate::Error { msg: String::new(), file: String::new(), line: 0 })?;
                if matches!(p.peek(), T::Punct("," | ")" | ";")) {
                    Ok(t)
                } else {
                    Err(crate::Error { msg: String::new(), file: String::new(), line: 0 })
                }
            }) {
                return Ok(Expr::new(ExprKind::Type(Box::new(t)), loc));
            }
        }
        self.mintypmax()
    }

    /// `#( ... )` parameter port list of a module, interface, class.
    pub(crate) fn param_port_list(&mut self) -> Result<Vec<ParamDecl>> {
        self.expect_p("#")?;
        self.expect_p("(")?;
        let mut v: Vec<ParamDecl> = Vec::new();
        if self.eat_p(")") {
            return Ok(v);
        }
        loop {
            let loc = self.loc();
            if matches!(self.kw(), Some(Kw::Parameter | Kw::Localparam)) {
                v.push(self.param_decl(false)?);
            } else if self.eat_kw(Kw::Type) {
                let assigns = self.type_param_assigns()?;
                v.push(ParamDecl { kw: None, kind: ParamKind::Type { assigns }, loc });
            } else if matches!(v.last(), Some(ParamDecl { kind: ParamKind::Type { .. }, .. })) && self.is_ident() && !self.is_type_start() {
                // `type A = int, B = bit`: still type parameters
                let more = self.type_param_assigns()?;
                if let Some(ParamDecl { kind: ParamKind::Type { assigns }, .. }) = v.last_mut() {
                    assigns.extend(more);
                }
            } else {
                let ty = self.param_value_type()?;
                let assigns = self.param_assigns()?;
                let implicit = matches!(ty.kind, TypeKind::Implicit) && ty.signed.is_none() && ty.packed.is_empty();
                match v.last_mut() {
                    Some(ParamDecl { kind: ParamKind::Value { assigns: prev, .. }, .. }) if implicit => prev.extend(assigns),
                    _ => v.push(ParamDecl { kw: None, kind: ParamKind::Value { ty, assigns }, loc }),
                }
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(v)
    }

    /// `typedef ...;` (the name becomes a type name).
    pub(crate) fn typedef(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::Typedef)?;
        // forward declarations
        let fwd = match (self.peek(), self.peek_at(1), self.peek_at(2), self.peek_at(3)) {
            (_, T::Punct(";"), _, _) if self.is_ident() => Some((None, false)),
            (T::Kw(k @ (Kw::Enum | Kw::Struct | Kw::Union | Kw::Class)), _, T::Punct(";"), _) if self.is_ident_at(1) => Some((Some(*k), false)),
            (T::Kw(Kw::Interface), T::Kw(Kw::Class), _, T::Punct(";")) => Some((Some(Kw::Class), true)),
            _ => None,
        };
        if let Some((kind, interface_class)) = fwd {
            if kind.is_some() {
                self.bump();
            }
            if interface_class {
                self.bump();
            }
            let name = self.ident()?;
            self.expect_p(";")?;
            self.add_type(&name);
            return Ok(Item::Typedef(Typedef::Forward { kind, interface_class, name, loc }));
        }
        // `typedef bus.data_t t;`: a type from an interface port
        if self.is_plain_ident_at(0) && self.is_p_at(1, ".") && self.is_ident_at(2) && self.is_ident_at(3) && self.is_p_at(4, ";") {
            let iface = self.ident()?;
            self.bump();
            let ty = self.ident()?;
            let name = self.ident()?;
            self.expect_p(";")?;
            self.add_type(&name);
            return Ok(Item::Typedef(Typedef::IfaceType { iface, ty, name, loc }));
        }
        let ty = match self.data_type()? {
            Some(t) => t,
            None if self.is_ident() && self.is_ident_at(1) => {
                // a type name the parser does not know
                let l = self.loc();
                let n = self.type_name()?;
                let signed = self.signing();
                let packed = self.packed_dims()?;
                DataType { kind: TypeKind::Named(Box::new(n)), signed, packed, loc: l }
            }
            None => return self.err(format!("expected a type after typedef, found {}", self.describe())),
        };
        let name = self.ident()?;
        let dims = self.unpacked_dims()?;
        self.expect_p(";")?;
        self.add_type(&name);
        Ok(Item::Typedef(Typedef::Type { ty, name, dims, loc }))
    }

    /// `import p::*, q::x;`
    pub(crate) fn import_decl(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::Import)?;
        let mut v = Vec::new();
        loop {
            let iloc = self.loc();
            let package = self.ident()?;
            self.expect_p("::")?;
            let name = if self.eat_p("*") { None } else { Some(self.member_name()?) };
            self.import_types(&package, name.as_deref());
            v.push(Import { package, name, loc: iloc });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")?;
        Ok(Item::Import(v, loc))
    }

    /// `import "DPI-C" [context|pure] [c_name =] function proto;` / `export "DPI-C" ...;`
    fn dpi(&mut self) -> Result<DpiDecl> {
        let loc = self.loc();
        let import = self.is_kw(Kw::Import);
        self.bump();
        let T::Str(spec) = self.bump() else { unreachable!() };
        let property = if self.is_kw(Kw::Context) || self.is_kw(Kw::Pure) { self.kw().inspect(|_| { self.bump(); }) } else { None };
        let c_name = if self.is_ident() && self.is_p_at(1, "=") {
            let n = self.ident()?;
            self.bump();
            Some(n)
        } else {
            None
        };
        let proto = if import {
            self.subroutine_proto()?
        } else {
            let sloc = self.loc();
            let is_task = self.is_kw(Kw::Task);
            if !self.eat_kw(Kw::Function) && !self.eat_kw(Kw::Task) {
                return self.err("expected 'function' or 'task'");
            }
            let n = self.ident()?;
            self.expect_p(";")?;
            Subroutine {
                is_task,
                quals: vec![],
                lifetime: None,
                ret: None,
                name: Name { scopes: vec![], name: n, params: None, loc: sloc },
                ports: None,
                body: None,
                loc: sloc,
            }
        };
        Ok(DpiDecl { import, spec, property, c_name, proto, loc })
    }

    /// A subroutine prototype `function int f(int a);` (no body).
    pub(crate) fn subroutine_proto(&mut self) -> Result<Subroutine> {
        let (mut s, _) = self.subroutine_head(vec![])?;
        self.expect_p(";")?;
        s.body = None;
        Ok(s)
    }

    /// The header of a function or task up to (not including) the `;`. Returns whether the
    /// name was `new`.
    fn subroutine_head(&mut self, quals: Vec<Kw>) -> Result<(Subroutine, bool)> {
        let loc = self.loc();
        let is_task = self.is_kw(Kw::Task);
        self.bump();
        let lifetime = if self.eat_kw(Kw::Automatic) {
            Some(Lifetime::Automatic)
        } else if self.eat_kw(Kw::Static) {
            Some(Lifetime::Static)
        } else {
            None
        };
        let mut ret = None;
        let name;
        if self.is_kw(Kw::New) {
            self.bump();
            name = Name { scopes: vec![], name: "new".into(), params: None, loc };
        } else if is_task {
            name = self.subroutine_name()?;
        } else {
            // a return type, or the name directly (implicit type)
            let save = self.pos;
            let t = self.data_type_or_implicit()?;
            let implicit_only = matches!(t.kind, TypeKind::Implicit) && t.signed.is_none() && t.packed.is_empty();
            if !implicit_only && (self.is_p("(") || self.is_p(";")) {
                // what looked like a type was the name (`function C::f(...)`, `function f;`
                // with `f` a type name elsewhere)
                if let TypeKind::Named(n) = t.kind {
                    if t.signed.is_none() && t.packed.is_empty() {
                        name = *n;
                        return self.subroutine_rest(is_task, quals, lifetime, None, name, loc);
                    }
                }
                self.pos = save;
                return self.err("expected a function name");
            }
            if !implicit_only {
                ret = Some(t);
            } else if t.signed.is_some() || !t.packed.is_empty() {
                ret = Some(t);
            }
            name = self.subroutine_name()?;
        }
        self.subroutine_rest(is_task, quals, lifetime, ret, name, loc)
    }

    fn subroutine_rest(
        &mut self,
        is_task: bool,
        quals: Vec<Kw>,
        lifetime: Option<Lifetime>,
        ret: Option<DataType>,
        name: Name,
        loc: Loc,
    ) -> Result<(Subroutine, bool)> {
        let is_new = name.name == "new";
        let ports = if self.is_p("(") { Some(self.tf_ports()?) } else { None };
        Ok((Subroutine { is_task, quals, lifetime, ret, name, ports, body: None, loc }, is_new))
    }

    /// `f`, `C::f`, `C::new`
    fn subroutine_name(&mut self) -> Result<Name> {
        let loc = self.loc();
        let mut scopes = Vec::new();
        let mut name = self.ident()?;
        while self.eat_p("::") {
            scopes.push(NamePart { name, params: None });
            name = if self.eat_kw(Kw::New) { "new".into() } else { self.ident()? };
        }
        Ok(Name { scopes, name, params: None, loc })
    }

    /// A function or task with its body (`extern`/`pure` prototypes end at the `;`).
    pub(crate) fn subroutine(&mut self, quals: Vec<Kw>) -> Result<Subroutine> {
        let prototype = quals.contains(&Kw::Extern) || quals.contains(&Kw::Pure);
        let (mut s, _) = self.subroutine_head(quals)?;
        self.expect_p(";")?;
        if prototype {
            return Ok(s);
        }
        let end = if s.is_task { Kw::Endtask } else { Kw::Endfunction };
        self.push_scope();
        let mut decls = Vec::new();
        let mut stmts = Vec::new();
        loop {
            if self.eat_kw(end) {
                break;
            }
            if self.at_eof() {
                return self.err(format!("unexpected end of file in {}", if s.is_task { "task" } else { "function" }));
            }
            if self.is_block_decl_start() {
                self.block_decl(&mut decls)?;
            } else {
                stmts.push(self.stmt()?);
            }
        }
        self.pop_scope();
        self.close_label(Some(&s.name.name))?;
        s.body = Some((decls, stmts));
        Ok(s)
    }

    /// `( [dir] [var] [type] name [dims] [= default], ... )`
    pub(crate) fn tf_ports(&mut self) -> Result<Vec<TfPort>> {
        self.expect_p("(")?;
        let mut v = Vec::new();
        if self.eat_p(")") {
            return Ok(v);
        }
        loop {
            let loc = self.loc();
            let dir = if self.is_kw(Kw::Const) && self.is_kw_at(1, Kw::Ref) {
                self.bump();
                self.bump();
                Some(Dir::ConstRef)
            } else {
                self.kw().and_then(dir_kw).inspect(|_| {
                    self.bump();
                })
            };
            let var = !self.soft_name_here() && self.eat_kw(Kw::Var);
            // property/sequence ports: `local input`, `untyped`, `sequence`, `property`
            self.eat_kw(Kw::Local);
            let ty = if self.is_ident() && !self.is_type_start() && matches!(self.peek_at(1), T::Punct("," | ")" | "=" | "[")) && !self.is_ident_at(1) {
                None
            } else if self.soft_name_here() {
                None
            } else if matches!(self.kw(), Some(Kw::Untyped | Kw::Sequence | Kw::Property)) {
                let l = self.loc();
                self.bump();
                Some(DataType { kind: TypeKind::Implicit, signed: None, packed: vec![], loc: l })
            } else {
                Some(self.decl_type()?)
            };
            let ty = match ty {
                // `input [3:0] a`: the dimensions belong to the type; nothing at all: inherit
                Some(t) if matches!(t.kind, TypeKind::Implicit) && t.signed.is_none() && t.packed.is_empty() && dir.is_none() && !var => None,
                t => t,
            };
            let name = self.ident()?;
            let dims = self.unpacked_dims()?;
            let default = if self.eat_p("=") { Some(self.expr()?) } else { None };
            v.push(TfPort { dir, var, ty, name, dims, default, loc });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(v)
    }

    /// `assign [strength] [#delay] a = b, c = d;`
    fn cont_assign(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::Assign)?;
        let strength = self.strength()?;
        let delay = if self.eat_p("#") { Some(self.delay_value()?) } else { None };
        let mut assigns = Vec::new();
        loop {
            let l = self.loc();
            let lhs = self.postfix_expr()?;
            self.expect_p("=")?;
            let rhs = self.expr()?;
            assigns.push((lhs, rhs, l));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")?;
        Ok(Item::ContAssign(ContAssign { strength, delay, assigns, loc }))
    }

    /// `timeunit 1ns [/ 1ps];` / `timeprecision 1ps;`
    fn timeunits(&mut self) -> Result<Item> {
        let loc = self.loc();
        let unit = self.is_kw(Kw::Timeunit);
        self.bump();
        let one = |p: &mut Parser| -> Result<(f64, i32)> {
            match p.bump() {
                T::Time(v, e) => Ok((v, e)),
                _ => {
                    p.pos -= 1;
                    p.err("expected a time literal")
                }
            }
        };
        let first = one(self)?;
        let (u, pr) = if unit {
            let p = if self.eat_p("/") { Some(one(self)?) } else { None };
            (Some(first), p)
        } else {
            (None, Some(first))
        };
        self.expect_p(";")?;
        Ok(Item::Timeunits { unit: u, precision: pr, loc })
    }

    /// `modport a (input x, output y), b (...);`
    fn modports(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::Modport)?;
        let mut v = Vec::new();
        loop {
            let mloc = self.loc();
            let name = self.ident()?;
            self.expect_p("(")?;
            let mut ports = Vec::new();
            let mut dir = Dir::Input;
            let mut tf: Option<bool> = None;
            if !self.is_p(")") {
                loop {
                    if let Some(d) = self.kw().and_then(dir_kw) {
                        self.bump();
                        dir = d;
                        tf = None;
                    } else if self.eat_kw(Kw::Import) {
                        tf = Some(true);
                    } else if self.eat_kw(Kw::Export) {
                        tf = Some(false);
                    } else if self.eat_kw(Kw::Clocking) {
                        ports.push(ModportPort::Clocking(self.ident()?));
                        if !self.eat_p(",") {
                            break;
                        }
                        continue;
                    }
                    if let Some(import) = tf {
                        if matches!(self.kw(), Some(Kw::Function | Kw::Task)) {
                            let (s, _) = self.subroutine_head(vec![])?;
                            let name = s.name.name.clone();
                            ports.push(ModportPort::Tf { import, name, proto: Some(Box::new(s)) });
                        } else {
                            ports.push(ModportPort::Tf { import, name: self.ident()?, proto: None });
                        }
                    } else if self.eat_p(".") {
                        let name = self.ident()?;
                        self.expect_p("(")?;
                        let e = if self.is_p(")") { None } else { Some(self.expr()?) };
                        self.expect_p(")")?;
                        ports.push(ModportPort::Simple { dir, name, expr: e });
                    } else {
                        ports.push(ModportPort::Simple { dir, name: self.ident()?, expr: None });
                    }
                    if !self.eat_p(",") {
                        break;
                    }
                }
            }
            self.expect_p(")")?;
            v.push(Modport { name, ports, loc: mloc });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")?;
        Ok(Item::Modport(v, loc))
    }

    /// Gate instances: `and #2 g1 (o, a, b), g2 (...);`
    fn gates(&mut self) -> Result<Item> {
        let loc = self.loc();
        let T::Kw(kind) = self.bump() else { unreachable!() };
        let strength = self.strength()?;
        let delay = if self.eat_p("#") { Some(self.delay_value()?) } else { None };
        let mut insts = Vec::new();
        loop {
            let l = self.loc();
            let name = if self.is_ident() { Some(self.ident()?) } else { None };
            let dims = self.unpacked_dims()?;
            self.expect_p("(")?;
            let mut terms = Vec::new();
            if !self.is_p(")") {
                loop {
                    terms.push(self.expr()?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
            }
            self.expect_p(")")?;
            insts.push(GateInst { name, dims, terms, loc: l });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")?;
        Ok(Item::Gates(Gates { kind, strength, delay, insts, loc }))
    }

    /// Module / interface / program / UDP / checker instances.
    pub(crate) fn instances(&mut self) -> Result<Item> {
        let loc = self.loc();
        let module = self.ident()?;
        let params = if self.is_p("#") { Some(self.param_values()?) } else { None };
        let mut insts = Vec::new();
        loop {
            let l = self.loc();
            let name = if self.is_p("(") { None } else { Some(self.ident()?) };
            let dims = self.unpacked_dims()?;
            self.expect_p("(")?;
            let conns = self.port_conns()?;
            insts.push(Instance { name, dims, conns, loc: l });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")?;
        Ok(Item::Instances(Instances { module, params, insts, loc }))
    }

    /// After `(`: port connections up to and including `)`.
    fn port_conns(&mut self) -> Result<Vec<PortConn>> {
        let mut v = Vec::new();
        if self.eat_p(")") {
            return Ok(v);
        }
        loop {
            if self.eat_p(".") {
                if self.eat_p("*") {
                    v.push(PortConn::Wildcard);
                } else {
                    let name = self.ident()?;
                    if self.eat_p("(") {
                        let e = if self.is_p(")") { None } else { Some(self.expr()?) };
                        self.expect_p(")")?;
                        v.push(PortConn::Named(name, Some(e)));
                    } else {
                        v.push(PortConn::Named(name, None));
                    }
                }
            } else if self.is_p(",") || self.is_p(")") {
                v.push(PortConn::Positional(None));
            } else {
                v.push(PortConn::Positional(Some(self.expr()?)));
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(v)
    }

    // ---------------------------------------------------------------- generate

    /// A generate block: `[begin [: name] items end [: name]]` or a single item.
    pub(crate) fn gen_block(&mut self) -> Result<GenBlock> {
        self.in_generate += 1;
        let b = self.gen_block_inner();
        self.in_generate -= 1;
        b
    }

    fn gen_block_inner(&mut self) -> Result<GenBlock> {
        let loc = self.loc();
        let label = self.block_label.take();
        if self.eat_kw(Kw::Begin) {
            let name = self.end_label()?;
            let mut items = Vec::new();
            while !self.eat_kw(Kw::End) {
                if self.at_eof() {
                    return self.err("unexpected end of file in generate block");
                }
                self.module_item(&mut items)?;
            }
            self.close_label(name.as_deref().or(label.as_deref()))?;
            return Ok(GenBlock { name, begin: true, items, loc });
        }
        // `label : begin`
        if self.is_ident() && self.is_p_at(1, ":") && self.is_kw_at(2, Kw::Begin) {
            let label = self.ident()?;
            self.bump();
            self.block_label = Some(label.clone());
            let mut b = self.gen_block_inner()?;
            if b.name.is_none() {
                b.name = Some(label);
            }
            return Ok(b);
        }
        let mut items = Vec::new();
        self.module_item(&mut items)?;
        Ok(GenBlock { name: None, begin: false, items, loc })
    }

    fn gen_for(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::For)?;
        self.expect_p("(")?;
        let genvar = self.eat_kw(Kw::Genvar);
        let var = self.ident()?;
        self.expect_p("=")?;
        let init = self.expr()?;
        self.expect_p(";")?;
        let cond = self.expr()?;
        self.expect_p(";")?;
        let step = self.assign_like()?;
        self.expect_p(")")?;
        let body = self.gen_block()?;
        Ok(Item::GenFor(Box::new(GenFor { genvar, var, init, cond, step, body, loc })))
    }

    fn gen_if(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::If)?;
        self.expect_p("(")?;
        let cond = self.expr()?;
        self.expect_p(")")?;
        let then_ = self.gen_block()?;
        let else_ = if self.eat_kw(Kw::Else) {
            if self.is_kw(Kw::If) {
                let l = self.loc();
                let i = self.gen_if()?;
                Some(GenBlock { name: None, begin: false, items: vec![i], loc: l })
            } else {
                Some(self.gen_block()?)
            }
        } else {
            None
        };
        Ok(Item::GenIf(Box::new(GenIf { cond, then_, else_, loc })))
    }

    fn gen_case(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::Case)?;
        self.expect_p("(")?;
        let expr = self.expr()?;
        self.expect_p(")")?;
        let mut arms = Vec::new();
        while !self.eat_kw(Kw::Endcase) {
            if self.at_eof() {
                return self.err("unexpected end of file in case");
            }
            let labels = if self.eat_kw(Kw::Default) {
                self.eat_p(":");
                vec![]
            } else {
                let mut l = vec![self.expr()?];
                while self.eat_p(",") {
                    l.push(self.expr()?);
                }
                self.expect_p(":")?;
                l
            };
            arms.push((labels, self.gen_block()?));
        }
        Ok(Item::GenCase(Box::new(GenCase { expr, arms, loc })))
    }

    // ---------------------------------------------------------------- specify

    fn specify(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::Specify)?;
        let mut v = Vec::new();
        while !self.eat_kw(Kw::Endspecify) {
            if self.at_eof() {
                return self.err("unterminated specify");
            }
            let l = self.loc();
            match self.peek().clone() {
                T::Kw(Kw::Specparam) => {
                    let p = self.param_decl(true)?;
                    self.expect_p(";")?;
                    v.push(SpecifyItem::Specparam(p));
                }
                T::Kw(k @ (Kw::PulsestyleOnevent | Kw::PulsestyleOndetect | Kw::Showcancelled | Kw::Noshowcancelled)) => {
                    self.bump();
                    let mut names = Vec::new();
                    loop {
                        let e = self.postfix_expr()?;
                        names.push(format!("{e:?}"));
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p(";")?;
                    v.push(SpecifyItem::Pulse { kw: k, names, loc: l });
                }
                T::SysIdent(name) => {
                    self.bump();
                    self.expect_p("(")?;
                    let mut args = Vec::new();
                    loop {
                        if self.is_p(",") || self.is_p(")") {
                            args.push(None);
                        } else {
                            let edge = if self.eat_kw(Kw::Edge) {
                                if self.is_p("[") {
                                    self.skip_balanced()?;
                                }
                                Some(Edge::Both)
                            } else {
                                self.edge()
                            };
                            let expr = self.mintypmax()?;
                            let cond = if self.is_triple_and() {
                                self.bump();
                                self.bump();
                                Some(self.expr()?)
                            } else {
                                None
                            };
                            args.push(Some(TimingCheckArg { edge, expr, cond }));
                        }
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p(")")?;
                    self.expect_p(";")?;
                    v.push(SpecifyItem::TimingCheck { name, args, loc: l });
                }
                T::Kw(Kw::If) | T::Kw(Kw::Ifnone) | T::Punct("(") => v.push(SpecifyItem::Path(Box::new(self.spec_path()?))),
                _ => return self.err(format!("unexpected {} in specify block", self.describe())),
            }
        }
        Ok(Item::Specify(v, loc))
    }

    fn spec_path(&mut self) -> Result<SpecPath> {
        let loc = self.loc();
        let cond = if self.eat_kw(Kw::If) {
            self.expect_p("(")?;
            let c = self.expr()?;
            self.expect_p(")")?;
            Some(Some(c))
        } else if self.eat_kw(Kw::Ifnone) {
            Some(None)
        } else {
            None
        };
        self.expect_p("(")?;
        let edge = self.edge();
        let mut srcs = vec![self.expr()?];
        while self.eat_p(",") {
            srcs.push(self.expr()?);
        }
        let (polarity, full) = match self.bump() {
            T::Punct("=>") => (None, false),
            T::Punct("*>") => (None, true),
            T::Punct("+=>") => (Some('+'), false),
            T::Punct("-=>") => (Some('-'), false),
            T::Punct("+*>") => (Some('+'), true),
            T::Punct("-*>") => (Some('-'), true),
            T::Punct(p @ ("+" | "-")) => match self.bump() {
                T::Punct("=>") => (p.chars().next(), false),
                T::Punct("*>") => (p.chars().next(), true),
                _ => return self.err("expected '=>' or '*>' in a path"),
            },
            _ => {
                self.pos -= 1;
                return self.err(format!("expected '=>' or '*>' in a path, found {}", self.describe()));
            }
        };
        let mut data = None;
        let dsts = if self.eat_p("(") {
            let mut d = vec![self.expr_no_cond()?];
            while self.eat_p(",") {
                d.push(self.expr_no_cond()?);
            }
            if self.eat_p("+:") || self.eat_p("-:") || self.eat_p(":") {
                data = Some(self.expr()?);
            }
            self.expect_p(")")?;
            d
        } else {
            let mut d = vec![self.expr()?];
            while self.eat_p(",") {
                d.push(self.expr()?);
            }
            d
        };
        self.expect_p(")")?;
        self.expect_p("=")?;
        let delays = if self.is_p("(") { self.delay_value()? } else { Delay::Value(self.primary()?) };
        self.expect_p(";")?;
        Ok(SpecPath { cond, edge, srcs, full, polarity, dsts, data, delays, loc })
    }

    /// A destination of an edge-sensitive path (`q` in `(q +: d)`): a name with selects.
    fn expr_no_cond(&mut self) -> Result<Expr> {
        self.postfix_expr()
    }

    // ---------------------------------------------------------------- clocking

    /// `clocking [name] @(ev); items endclocking` (after `default`/`global`).
    fn clocking(&mut self, kind: Option<Kw>) -> Result<ClockingDecl> {
        let loc = self.loc();
        self.expect_kw(Kw::Clocking)?;
        let name = if self.is_ident() { Some(self.ident()?) } else { None };
        self.expect_p("@")?;
        let event = self.event_control()?;
        self.expect_p(";")?;
        let mut items = Vec::new();
        while !self.eat_kw(Kw::Endclocking) {
            if self.at_eof() {
                return self.err("unexpected end of file in clocking block");
            }
            if self.eat_kw(Kw::Default) {
                let mut input = None;
                let mut output = None;
                loop {
                    if self.eat_kw(Kw::Input) {
                        input = Some(self.skew()?);
                    } else if self.eat_kw(Kw::Output) {
                        output = Some(self.skew()?);
                    } else {
                        break;
                    }
                }
                self.expect_p(";")?;
                items.push(ClockingItem::Default { input, output });
                continue;
            }
            if let Some(dir) = self.kw().and_then(dir_kw) {
                self.bump();
                let mut input = None;
                let mut output = None;
                if dir == Dir::Input || dir == Dir::Inout {
                    if dir == Dir::Input {
                        input = Some(self.skew()?);
                    }
                    if self.eat_kw(Kw::Output) {
                        output = Some(self.skew()?);
                    }
                } else {
                    output = Some(self.skew()?);
                }
                let mut names = Vec::new();
                loop {
                    let n = self.ident()?;
                    let e = if self.eat_p("=") { Some(self.expr()?) } else { None };
                    names.push((n, e));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                items.push(ClockingItem::Signals { dir, input, output, names });
                continue;
            }
            let mut v = Vec::new();
            self.module_item(&mut v)?;
            items.extend(v.into_iter().map(ClockingItem::Item));
        }
        self.close_label(name.as_deref())?;
        Ok(ClockingDecl { kind, name, event, items, loc })
    }

    /// `[posedge|negedge|edge] [#delay]` of a clocking direction.
    fn skew(&mut self) -> Result<Skew> {
        let edge = self.edge();
        let delay = if self.eat_p("#") {
            Some(match self.delay_value()? {
                Delay::Value(e) => e,
                Delay::List(mut v) => v.remove(0),
            })
        } else {
            None
        };
        Ok(Skew { edge, delay })
    }

    // ---------------------------------------------------------------- misc

    /// `let name [(ports)] = expr;`
    pub(crate) fn let_decl(&mut self) -> Result<Item> {
        let loc = self.loc();
        self.expect_kw(Kw::Let)?;
        let name = self.ident()?;
        let ports = if self.is_p("(") { self.tf_ports()? } else { vec![] };
        self.expect_p("=")?;
        let expr = self.expr()?;
        self.expect_p(";")?;
        Ok(Item::Let(Box::new(LetDecl { name, ports, expr, loc })))
    }

    /// `bind target [: inst, inst] module #(...) name (...);`
    fn bind(&mut self) -> Result<Bind> {
        let loc = self.loc();
        self.expect_kw(Kw::Bind)?;
        let target = self.postfix_expr()?;
        let mut targets = Vec::new();
        if self.eat_p(":") {
            loop {
                targets.push(self.postfix_expr()?);
                if !self.eat_p(",") {
                    break;
                }
            }
        }
        let Item::Instances(inst) = self.instances()? else { unreachable!() };
        Ok(Bind { target, targets, inst, loc })
    }

    /// A directive token from the preprocessor.
    pub(crate) fn directive(&mut self) -> Result<Directive> {
        let T::Directive(d) = self.bump() else { unreachable!() };
        let r = match d.as_str() {
            "__fx_timescale" => {
                let num = |p: &mut Parser| -> Result<i32> {
                    let neg = p.eat_p("-");
                    match p.bump() {
                        T::Num(n) => {
                            let v: i32 = n.digits.parse().unwrap_or(0);
                            Ok(if neg { -v } else { v })
                        }
                        _ => p.err("bad timescale"),
                    }
                };
                let u = num(self)?;
                let p = num(self)?;
                Directive::Timescale(u, p)
            }
            "__fx_nettype" => {
                let t = match self.bump() {
                    T::Ident(s) => s,
                    T::Kw(k) => k.as_str().to_string(),
                    _ => return self.err("bad `default_nettype"),
                };
                Directive::DefaultNettype(t)
            }
            "__fx_uncdrive" => {
                let t = match self.bump() {
                    T::Ident(s) => s,
                    T::Kw(k) => k.as_str().to_string(),
                    _ => return self.err("bad `unconnected_drive"),
                };
                Directive::UnconnectedDrive(match t.as_str() {
                    "pull1" => Some(true),
                    "pull0" => Some(false),
                    _ => None,
                })
            }
            "__fx_keywords" => match self.peek() {
                T::Str(_) => {
                    let T::Str(v) = self.bump() else { unreachable!() };
                    Directive::Keywords(Some(v))
                }
                _ => Directive::Keywords(None),
            },
            _ => return self.err(format!("unknown directive {d}")),
        };
        self.expect_p(";")?;
        Ok(r)
    }
}
