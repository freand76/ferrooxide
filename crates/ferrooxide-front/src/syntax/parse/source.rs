//! Source text: modules and their headers, packages, primitives, configurations (IEEE 1800
//! A.1).

use super::items::{dir_kw, net_kw};
use super::Parser;
use crate::syntax::kw::Kw;
use crate::syntax::token::T;
use crate::syntax::tree::*;
use crate::Result;

impl Parser {
    pub(crate) fn source_text(&mut self) -> Result<Vec<Description>> {
        let mut v = Vec::new();
        while !self.at_eof() {
            let loc = self.loc();
            match self.peek().clone() {
                T::Directive(_) => {
                    let d = self.directive()?;
                    v.push(Description::Directive(d, loc));
                }
                T::Kw(Kw::Module | Kw::Macromodule | Kw::Program | Kw::Checker) => v.push(Description::Module(self.module_decl(false)?)),
                T::Kw(Kw::Interface) if !self.is_kw_at(1, Kw::Class) => v.push(Description::Module(self.module_decl(false)?)),
                T::Kw(Kw::Extern) if matches!(self.peek_at(1), T::Kw(Kw::Module | Kw::Macromodule | Kw::Interface | Kw::Program)) => {
                    self.bump();
                    v.push(Description::Module(self.module_decl(true)?));
                }
                T::Kw(Kw::Primitive) => v.push(Description::Udp(self.udp()?)),
                T::Kw(Kw::Package) => v.push(Description::Package(self.package()?)),
                T::Kw(Kw::Config) if self.is_ident_at(1) && self.is_p_at(2, ";") => v.push(Description::Config(self.config()?)),
                _ => {
                    let mut items = Vec::new();
                    self.module_item(&mut items)?;
                    v.extend(items.into_iter().map(Description::Item));
                }
            }
        }
        Ok(v)
    }

    /// `module ... endmodule` (also interface, program, checker); `extern`: the header only.
    pub(crate) fn module_decl(&mut self, extern_: bool) -> Result<ModuleDecl> {
        let loc = self.loc();
        let (kind, end) = match self.bump() {
            T::Kw(Kw::Module) => (ModuleKind::Module, Kw::Endmodule),
            T::Kw(Kw::Macromodule) => (ModuleKind::Macromodule, Kw::Endmodule),
            T::Kw(Kw::Interface) => (ModuleKind::Interface, Kw::Endinterface),
            T::Kw(Kw::Program) => (ModuleKind::Program, Kw::Endprogram),
            T::Kw(Kw::Checker) => (ModuleKind::Checker, Kw::Endchecker),
            _ => unreachable!(),
        };
        let lifetime = self.lifetime();
        let name = self.ident()?;
        self.push_scope();
        let mut imports = Vec::new();
        while self.is_kw(Kw::Import) {
            if let Item::Import(v, _) = self.import_decl()? {
                imports.extend(v);
            }
        }
        let params = if self.is_p("#") { Some(self.param_port_list()?) } else { None };
        let ports = if self.is_p("(") { self.port_list()? } else { Ports::None };
        self.expect_p(";")?;
        let mut items = Vec::new();
        if !extern_ {
            while !self.eat_kw(end) {
                if self.at_eof() {
                    return self.err(format!("unexpected end of file in {}", end.as_str().trim_start_matches("end")));
                }
                self.module_item(&mut items)?;
            }
            self.end_label()?;
        }
        self.pop_scope();
        Ok(ModuleDecl { kind, name, lifetime, extern_, imports, params, ports, items, loc })
    }

    pub(crate) fn lifetime(&mut self) -> Option<Lifetime> {
        if self.eat_kw(Kw::Static) {
            Some(Lifetime::Static)
        } else if self.eat_kw(Kw::Automatic) {
            Some(Lifetime::Automatic)
        } else {
            None
        }
    }

    /// `( ... )` of a module header: ANSI or non-ANSI.
    fn port_list(&mut self) -> Result<Ports> {
        self.expect_p("(")?;
        if self.eat_p(")") {
            return Ok(Ports::NonAnsi(vec![]));
        }
        if self.is_p(".") && self.is_p_at(1, "*") {
            self.bump();
            self.bump();
            self.expect_p(")")?;
            return Ok(Ports::Wildcard);
        }
        if self.is_ansi_port_start() {
            let mut v = Vec::new();
            loop {
                v.push(self.ansi_port(v.last())?);
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p(")")?;
            return Ok(Ports::Ansi(v));
        }
        let mut v = Vec::new();
        loop {
            let loc = self.loc();
            if self.is_p(",") || self.is_p(")") {
                v.push(PortExpr { name: None, expr: None, loc });
            } else if self.eat_p(".") {
                let name = self.ident()?;
                self.expect_p("(")?;
                let e = if self.is_p(")") { None } else { Some(self.expr()?) };
                self.expect_p(")")?;
                v.push(PortExpr { name: Some(name), expr: e, loc });
            } else {
                let e = self.expr()?;
                v.push(PortExpr { name: None, expr: Some(e), loc });
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(Ports::NonAnsi(v))
    }

    fn is_ansi_port_start(&self) -> bool {
        match self.peek() {
            T::Kw(k) if dir_kw(*k).is_some() || net_kw(*k).is_some() => true,
            T::Kw(Kw::Var | Kw::Interface | Kw::Const) => true,
            T::Kw(Kw::Signed | Kw::Unsigned) => true,
            T::Punct("[") => true,
            _ => (self.is_type_start() && !self.soft_name_here()) || self.is_iface_port(),
        }
    }

    /// `bus_if b` / `bus_if.master b` (a name that is no type, followed by a port name).
    fn is_iface_port(&self) -> bool {
        let T::Ident(t) = self.peek() else { return false };
        if self.is_type_name(t) || self.packages.contains_key(t.as_str()) {
            return false;
        }
        match (self.peek_at(1), self.peek_at(2), self.peek_at(3)) {
            (T::Punct("."), _, _) if self.is_ident_at(2) && self.is_ident_at(3) => true,
            (_, T::Punct("," | ")" | "["), _) if matches!(self.peek_at(1), T::Ident(_)) => true,
            _ => false,
        }
    }

    /// One ANSI port; `prev`: the port before (for an interface port's further names).
    fn ansi_port(&mut self, prev: Option<&AnsiPort>) -> Result<AnsiPort> {
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
        // `.name(expr)`
        if self.eat_p(".") {
            let name = self.ident()?;
            self.expect_p("(")?;
            let e = if self.is_p(")") { None } else { Some(self.expr()?) };
            self.expect_p(")")?;
            return Ok(AnsiPort { dir, header: PortHeader::None, name, dims: vec![], default: None, expr: Some(e), loc });
        }
        let header = if self.is_kw(Kw::Interface) {
            self.bump();
            let modport = if self.eat_p(".") { Some(self.ident()?) } else { None };
            PortHeader::Interface { name: None, modport }
        } else if dir.is_none() && self.is_iface_port() {
            let name = self.ident()?;
            let modport = if self.eat_p(".") { Some(self.ident()?) } else { None };
            PortHeader::Interface { name: Some(name), modport }
        } else {
            let net = self.kw().and_then(net_kw).inspect(|_| {
                self.bump();
            });
            let var = !self.soft_name_here() && self.eat_kw(Kw::Var);
            // nothing but the name: everything inherited
            let bare = net.is_none() && !var && self.is_ident() && !self.is_type_start() && matches!(self.peek_at(1), T::Punct("," | ")" | "[" | "="));
            // a further name after an interface port
            let _ = prev;
            if bare {
                PortHeader::None
            } else {
                let ty = self.decl_type()?;
                PortHeader::Data { net, var, ty }
            }
        };
        let name = self.ident()?;
        let dims = self.unpacked_dims()?;
        let default = if self.eat_p("=") { Some(self.expr()?) } else { None };
        Ok(AnsiPort { dir, header, name, dims, default, expr: None, loc })
    }

    /// `package name; items endpackage` (its type names are recorded for `p::T` and imports).
    fn package(&mut self) -> Result<PackageDecl> {
        let loc = self.loc();
        self.expect_kw(Kw::Package)?;
        let lifetime = self.lifetime();
        let name = self.ident()?;
        self.expect_p(";")?;
        self.push_scope();
        let mut items = Vec::new();
        while !self.eat_kw(Kw::Endpackage) {
            if self.at_eof() {
                return self.err("unexpected end of file in package");
            }
            self.module_item(&mut items)?;
        }
        self.end_label()?;
        let types = self.pop_scope();
        self.packages.insert(name.clone(), types);
        Ok(PackageDecl { name, lifetime, items, loc })
    }

    /// `primitive ... endprimitive`
    fn udp(&mut self) -> Result<UdpDecl> {
        let loc = self.loc();
        self.expect_kw(Kw::Primitive)?;
        let name = self.ident()?;
        let mut ports = Vec::new();
        let mut decls = Vec::new();
        self.expect_p("(")?;
        loop {
            if let Some(dir) = self.kw().and_then(dir_kw) {
                let ploc = self.loc();
                self.bump();
                let ty = self.decl_type()?;
                let dl = self.loc();
                let n = self.ident()?;
                let init = if self.eat_p("=") { Some(self.expr()?) } else { None };
                ports.push(n.clone());
                decls.push(Item::Port(PortDecl { dir, net: None, var: false, ty, decls: vec![Declarator { name: n, dims: vec![], init, loc: dl }], loc: ploc }));
            } else {
                ports.push(self.ident()?);
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        self.expect_p(";")?;
        let mut rows = Vec::new();
        let mut init = None;
        loop {
            if self.eat_kw(Kw::Endprimitive) {
                break;
            }
            if self.at_eof() {
                return self.err("unexpected end of file in primitive");
            }
            if self.eat_kw(Kw::Initial) {
                let n = self.ident()?;
                self.expect_p("=")?;
                init = Some((n, self.expr()?));
                self.expect_p(";")?;
            } else if self.eat_kw(Kw::Table) {
                let raw = match self.bump() {
                    T::Str(s) => s,
                    _ => return self.err("bad table"),
                };
                self.expect_kw(Kw::Endtable)?;
                for row in raw.split(';') {
                    let row = row.trim();
                    if row.is_empty() {
                        continue;
                    }
                    let parts: Vec<&str> = row.split(':').collect();
                    let syms = |t: &str| -> Vec<String> {
                        let mut v = Vec::new();
                        let cs: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
                        let mut k = 0;
                        while k < cs.len() {
                            if cs[k] == '(' {
                                let e: String = cs[k..(k + 4).min(cs.len())].iter().collect();
                                v.push(e.to_ascii_lowercase());
                                k += 4;
                            } else {
                                v.push(cs[k].to_ascii_lowercase().to_string());
                                k += 1;
                            }
                        }
                        v
                    };
                    match parts.len() {
                        2 => rows.push((syms(parts[0]), None, parts[1].trim().to_ascii_lowercase())),
                        3 => rows.push((syms(parts[0]), Some(parts[1].trim().to_ascii_lowercase()), parts[2].trim().to_ascii_lowercase())),
                        _ => return self.err(format!("bad UDP table row '{row}'")),
                    }
                }
            } else if self.is_decl_start() {
                decls.push(self.data_decl()?);
            } else {
                return self.err(format!("unexpected {} in primitive", self.describe()));
            }
        }
        self.end_label()?;
        Ok(UdpDecl { name, ports, decls, init, rows, loc })
    }

    /// `config name; design ...; rules endconfig` (rules kept as text).
    fn config(&mut self) -> Result<ConfigDecl> {
        let loc = self.loc();
        self.expect_kw(Kw::Config)?;
        let name = self.ident()?;
        self.expect_p(";")?;
        let mut design = Vec::new();
        let mut rules = Vec::new();
        while !self.eat_kw(Kw::Endconfig) {
            if self.at_eof() {
                return self.err("unexpected end of file in config");
            }
            if self.eat_kw(Kw::Design) {
                while !self.eat_p(";") {
                    let mut n = self.ident()?;
                    if self.eat_p(".") {
                        n.push('.');
                        n.push_str(&self.ident()?);
                    }
                    design.push(n);
                }
                continue;
            }
            let mut text = String::new();
            while !self.eat_p(";") {
                if self.at_eof() {
                    return self.err("unexpected end of file in config");
                }
                let t = self.bump();
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&super::describe(&t).trim_matches('\''));
            }
            rules.push(text);
        }
        self.end_label()?;
        Ok(ConfigDecl { name, design, rules, loc })
    }
}
