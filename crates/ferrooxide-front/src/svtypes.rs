//! SystemVerilog data types in the parser: `typedef`, `enum`, packed `struct`/`union`, casts
//! and assignment patterns. Types are resolved here, so elaboration only sees vectors:
//!
//! - a type without a struct becomes the declaration's kind, sign and packed ranges;
//! - a type containing a struct is declared as one flat vector `[W-1:0]`, and every access
//!   chain (`v[i].f.g[3:0]`) becomes one select `v[off +: w]` (offsets are expressions, so
//!   parameterized widths and run-time indexes work);
//! - enum constants become `localparam`s of the base type;
//! - `T'(e)`, `N'(e)` are `$__cast(width, sign, e)` (sign 0, 1 or 2: keep);
//! - `'{...}` is resolved against the target type into a concatenation of casts.

use crate::ast::*;
use crate::lexer::Tok;
use crate::parser::{dec, item_name, Parser};
use crate::Result;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct SvType {
    pub kind: NetKind,
    pub signed: bool,
    /// packed dimensions, outermost first; integer types have their implicit [n-1:0]
    pub dims: Vec<Range>,
    /// the element is a packed struct / union
    pub fields: Option<Rc<Struct>>,
    /// `int`, `integer`, `byte`, ...: declared without a range
    pub int_kind: bool,
    /// an enum: its constants in order
    pub enum_names: Option<Rc<Vec<String>>>,
    /// unpacked dimensions of a typedef (`typedef logic [7:0] T [4];`): a declaration of the
    /// type is an array with these dimensions after its own
    pub unpacked: Vec<Range>,
}

/// A package: its items (with those of the packages it imports) and its typedefs.
#[derive(Debug, Default)]
pub struct Package {
    /// items copied into the modules that use the package (no variables)
    pub items: Vec<Item>,
    /// its variables (in the package's own module) and their types
    pub vars: std::collections::HashMap<String, (Rc<SvType>, Vec<Range>)>,
    /// its functions and tasks (in the package's own module)
    pub funcs: std::collections::HashSet<String>,
    pub types: std::collections::HashMap<String, Rc<SvType>>,
}

#[derive(Debug)]
pub struct Struct {
    pub union: bool,
    pub fields: Vec<(String, SvType)>,
}

/// The value of a plain literal (no x/z), possibly negated.
fn lit(e: &Expr) -> Option<i64> {
    match e {
        Expr::Num(n) if !n.fill && !n.digits.contains(['x', 'z', 'X', 'Z', '?']) => {
            i64::from_str_radix(&n.digits.replace('_', ""), n.base as u32).ok()
        }
        Expr::Unary(UnOp::Neg, a) => lit(a).map(|v| -v),
        _ => None,
    }
}

fn num(v: i64) -> Expr {
    if v < 0 {
        Expr::Unary(UnOp::Neg, Box::new(Expr::Num(dec(-v as u64))))
    } else {
        Expr::Num(dec(v as u64))
    }
}

fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
    if let (Some(x), Some(y)) = (lit(&a), lit(&b)) {
        match op {
            BinOp::Add => return num(x + y),
            BinOp::Sub => return num(x - y),
            BinOp::Mul => return num(x * y),
            _ => {}
        }
    }
    match (op, lit(&a), lit(&b)) {
        (BinOp::Add, Some(0), _) => return b,
        (BinOp::Add | BinOp::Sub, _, Some(0)) => return a,
        (BinOp::Mul, Some(1), _) => return b,
        (BinOp::Mul, _, Some(1)) => return a,
        _ => {}
    }
    Expr::Binary(op, Box::new(a), Box::new(b))
}

fn add(a: Expr, b: Expr) -> Expr {
    bin(BinOp::Add, a, b)
}
fn sub(a: Expr, b: Expr) -> Expr {
    bin(BinOp::Sub, a, b)
}
fn mul(a: Expr, b: Expr) -> Expr {
    bin(BinOp::Mul, a, b)
}

/// Number of elements of a dimension [a:b].
fn dim_size(r: &Range) -> Expr {
    if let (Some(a), Some(b)) = (lit(&r.0), lit(&r.1)) {
        return num((a - b).abs() + 1);
    }
    let ge = Expr::Binary(BinOp::Ge, Box::new(r.0.clone()), Box::new(r.1.clone()));
    let d = Expr::Cond(Box::new(ge), Box::new(sub(r.0.clone(), r.1.clone())), Box::new(sub(r.1.clone(), r.0.clone())));
    add(d, num(1))
}

/// Position of index `k` in dimension [msb:lsb], counted from the lsb side.
fn dim_pos(r: &Range, k: Expr) -> Expr {
    if let (Some(a), Some(b)) = (lit(&r.0), lit(&r.1)) {
        return if a >= b { sub(k, num(b)) } else { sub(num(b), k) };
    }
    let ge = Expr::Binary(BinOp::Ge, Box::new(r.0.clone()), Box::new(r.1.clone()));
    Expr::Cond(Box::new(ge), Box::new(sub(k.clone(), r.1.clone())), Box::new(sub(r.1.clone(), k)))
}

/// Whether a dimension counts down ([7:0]); unknown (parameterized) ones are assumed to.
fn descending(r: &Range) -> bool {
    match (lit(&r.0), lit(&r.1)) {
        (Some(a), Some(b)) => a >= b,
        _ => true,
    }
}

impl SvType {
    pub fn vector(kind: NetKind, signed: bool, dims: Vec<Range>) -> SvType {
        SvType { kind, signed, dims, fields: None, int_kind: false, enum_names: None, unpacked: vec![] }
    }

    /// Width of one element after the packed dimensions.
    pub fn leaf_width(&self) -> Expr {
        match &self.fields {
            None => num(1),
            Some(s) if s.union => s.fields.first().map_or(num(0), |f| f.1.width()),
            Some(s) => s.fields.iter().fold(num(0), |acc, f| add(acc, f.1.width())),
        }
    }

    /// Width of the dimensions from `from` on, times the element width.
    pub fn width_from(&self, from: usize) -> Expr {
        self.dims[from..].iter().fold(self.leaf_width(), |acc, d| mul(dim_size(d), acc))
    }

    pub fn width(&self) -> Expr {
        self.width_from(0)
    }

    /// `$bits` of the type: the packed width times the unpacked dimensions.
    pub fn bits(&self) -> Expr {
        self.unpacked.iter().fold(self.width(), |acc, d| mul(dim_size(d), acc))
    }

    pub fn has_struct(&self) -> bool {
        self.fields.is_some()
    }

    /// Declaration parts: kind, sign, range, further packed ranges.
    pub fn decl_parts(&self) -> (NetKind, bool, Option<Range>, Vec<Range>) {
        if self.has_struct() {
            let kind = if self.kind == NetKind::Bit { NetKind::Bit } else { NetKind::Reg };
            return (kind, self.signed, Some((sub(self.width(), num(1)), num(0))), vec![]);
        }
        if self.int_kind {
            return (self.kind, self.signed, None, vec![]);
        }
        let mut it = self.dims.iter().cloned();
        (self.kind, self.signed, it.next(), it.collect())
    }
}

/// Builtin type keywords: (kind, signed, width of an integer type).
fn builtin(s: &str) -> Option<(NetKind, bool, Option<u32>)> {
    Some(match s {
        "logic" | "reg" => (NetKind::Reg, false, None),
        "bit" => (NetKind::Bit, false, None),
        "int" => (NetKind::Int(32), true, Some(32)),
        "integer" => (NetKind::Integer, true, Some(32)),
        "byte" => (NetKind::Int(8), true, Some(8)),
        "shortint" => (NetKind::Int(16), true, Some(16)),
        "longint" => (NetKind::Int(64), true, Some(64)),
        "time" => (NetKind::Time, false, Some(64)),
        _ => return None,
    })
}

impl Parser {
    /// `package name; items endpackage`
    pub(crate) fn package(&mut self) -> Result<()> {
        self.bump();
        let name = self.ident()?;
        self.expect_p(";")?;
        let saved_types = self.types.clone();
        let saved_pkgs = std::mem::take(&mut self.mod_pkgs);
        let saved_vars = std::mem::take(&mut self.vars);
        let saved_tu = std::mem::replace(&mut self.local_tu, (None, None));
        let mut own = Vec::new();
        while !self.eat_kw("endpackage") {
            if matches!(self.peek(), Tok::Eof) {
                return self.err("unexpected end of file in package");
            }
            self.module_item(&mut own)?;
        }
        self.opt_block_label()?;
        // the package's typedefs: those added while parsing it
        let mut types = std::collections::HashMap::new();
        for (k, v) in &self.types {
            if saved_types.get(k).is_none_or(|o| !Rc::ptr_eq(o, v)) {
                types.insert(k.clone(), v.clone());
            }
        }
        // items of the packages it imports come first
        let used = std::mem::replace(&mut self.mod_pkgs, saved_pkgs);
        let mut items = self.package_items(&used);
        let mut var_items = Vec::new();
        for it in own {
            // variables, functions and tasks stay in the package's own scope
            if matches!(it, Item::Decl(_) | Item::Subroutine(_)) {
                var_items.push(it);
            } else {
                items.push(it);
            }
        }
        let funcs = var_items.iter().filter_map(|it| if let Item::Subroutine(f) = it { Some(f.name.clone()) } else { None }).collect();
        let mut vars = std::collections::HashMap::new();
        let declared = std::mem::replace(&mut self.vars, saved_vars);
        for it in &var_items {
            if let Item::Decl(d) = it {
                for n in &d.names {
                    if let Some(t) = declared.get(&n.name) {
                        vars.insert(n.name.clone(), t.clone());
                    }
                }
            }
        }
        {
            // (also without variables: the package is a scope of the design)
            let mut all = items.clone();
            all.extend(var_items);
            let m = self.package_module(name.clone(), all, used);
            self.pkg_modules.push(m);
        }
        self.local_tu = saved_tu;
        self.types = saved_types;
        self.packages.insert(name, Rc::new(Package { items, types, vars, funcs }));
        Ok(())
    }

    /// `import p::*, q::name;` (in a module, a package, or at file scope).
    pub(crate) fn import(&mut self, file_scope: bool) -> Result<()> {
        self.bump();
        loop {
            let pkg = self.ident()?;
            self.expect_p("::")?;
            let Some(p) = self.packages.get(&pkg).cloned() else {
                return self.err(format!("unknown package '{pkg}'"));
            };
            if self.eat_p("*") {
                for (k, v) in &p.types {
                    self.types.insert(k.clone(), v.clone());
                }
                for (k, v) in &p.vars {
                    self.vars.entry(k.clone()).or_insert_with(|| v.clone());
                    if file_scope {
                        self.unit_var_types.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            } else {
                let n = self.ident()?;
                if let Some(t) = p.types.get(&n) {
                    self.types.insert(n.clone(), t.clone());
                }
                if let Some(v) = p.vars.get(&n) {
                    self.vars.entry(n.clone()).or_insert_with(|| v.clone());
                    if file_scope {
                        self.unit_var_types.entry(n).or_insert_with(|| v.clone());
                    }
                }
            }
            if file_scope && !self.unit_pkgs.contains(&pkg) {
                self.unit_pkgs.push(pkg.clone());
            }
            if file_scope {
                let items = self.package_items(&[pkg]);
                let have: std::collections::HashSet<String> = self.unit_items.iter().filter_map(|i| item_name(i).map(str::to_string)).collect();
                self.unit_items.extend(items.into_iter().filter(|i| item_name(i).is_none_or(|n| !have.contains(n))));
            } else if !self.mod_pkgs.contains(&pkg) {
                self.mod_pkgs.push(pkg);
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(";")
    }

    /// `pkg::` at the cursor (a package-qualified name): consumes it, makes the package's
    /// items part of the current module and the name visible as a type if it is one.
    pub(crate) fn pkg_scope(&mut self) -> Result<bool> {
        let (Tok::Ident(pkg), Tok::Punct("::"), Tok::Ident(n)) = (self.peek().clone(), self.peek_at(1).clone(), self.peek_at(2).clone()) else {
            return Ok(false);
        };
        let Some(p) = self.packages.get(&pkg).cloned() else { return Ok(false) };
        self.bump();
        self.bump();
        if let Some(t) = p.types.get(&n) {
            self.types.insert(n, t.clone());
        }
        if !self.mod_pkgs.contains(&pkg) {
            self.mod_pkgs.push(pkg);
        }
        Ok(true)
    }

    /// The items of packages, each name once (the first package wins).
    pub(crate) fn package_items(&self, pkgs: &[String]) -> Vec<Item> {
        let mut seen = std::collections::HashSet::new();
        let mut v = Vec::new();
        for p in pkgs {
            if let Some(p) = self.packages.get(p) {
                for it in &p.items {
                    if item_name(it).is_none_or(|n| seen.insert(n.to_string())) {
                        v.push(it.clone());
                    }
                }
            }
        }
        v
    }

    /// The current token is a typedef name (not a call or an instance name).
    pub(crate) fn is_type_name(&self) -> bool {
        matches!(self.peek(), Tok::Ident(s) if self.types.contains_key(s))
            && !matches!(self.peek_at(1), Tok::Punct("(" | "#" | "." | "=" | "<=" | ";"))
    }

    /// A data type with a struct, union or enum, or a typedef name, starts here.
    pub(crate) fn is_sv_type_start(&self) -> bool {
        self.is_kw("struct") || self.is_kw("union") || self.is_kw("enum") || self.is_type_name() || self.is_pkg_type()
    }

    /// `pkg::type_name` at the cursor.
    fn is_pkg_type(&self) -> bool {
        self.is_pkg_type_at(0)
    }

    /// `pkg::type_name` `k` tokens ahead.
    pub(crate) fn is_pkg_type_at(&self, k: usize) -> bool {
        match (self.peek_at(k), self.peek_at(k + 1), self.peek_at(k + 2)) {
            (Tok::Ident(p), Tok::Punct("::"), Tok::Ident(n)) => self.packages.get(p).is_some_and(|p| p.types.contains_key(n)),
            _ => false,
        }
    }

    /// `[signed|unsigned]` after a type.
    fn signing(&mut self, default: bool) -> bool {
        if self.eat_kw("signed") {
            true
        } else if self.eat_kw("unsigned") {
            false
        } else {
            default
        }
    }

    /// A data type: builtin vector/integer types, struct/union, enum, or a typedef name, each
    /// with optional packed dimensions. None if no type starts here.
    pub(crate) fn data_type(&mut self) -> Result<Option<SvType>> {
        if self.is_pkg_type() {
            self.pkg_scope()?;
        }
        let Tok::Ident(s) = self.peek().clone() else { return Ok(None) };
        if s == "struct" || s == "union" {
            self.bump();
            let union = s == "union";
            self.eat_kw("tagged");
            self.eat_kw("packed");
            let signed = self.signing(false);
            self.expect_p("{")?;
            let mut fields = Vec::new();
            while !self.eat_p("}") {
                let Some(ft) = self.data_type()? else {
                    return self.err(format!("expected a member type, found {}", self.describe()));
                };
                loop {
                    let name = self.ident()?;
                    fields.push((name, ft.clone()));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
            }
            let mut dims = Vec::new();
            while self.is_p("[") {
                dims.push(self.range()?);
            }
            let kind = if fields.iter().all(|f| f.1.kind == NetKind::Bit) { NetKind::Bit } else { NetKind::Reg };
            return Ok(Some(SvType { kind, signed, dims, fields: Some(Rc::new(Struct { union, fields })), int_kind: false, enum_names: None, unpacked: vec![] }));
        }
        if s == "enum" {
            self.bump();
            let base = match self.data_type()? {
                Some(t) => t,
                None => {
                    let signed = self.signing(true);
                    SvType { kind: NetKind::Int(32), signed, dims: vec![(num(31), num(0))], fields: None, int_kind: true, enum_names: None, unpacked: vec![] }
                }
            };
            self.expect_p("{")?;
            let mut prev: Option<String> = None;
            let mut names = Vec::new();
            let (_, signed, range, _) = base.decl_parts();
            let range = range.or_else(|| base.int_kind.then(|| base.dims[0].clone()));
            loop {
                let line = self.line();
                let name = self.ident()?;
                let value = if self.eat_p("=") {
                    self.expr()?
                } else {
                    match &prev {
                        Some(p) => add(Expr::Ident(p.clone()), num(1)),
                        None => num(0),
                    }
                };
                self.pending.push(Item::Param(ParamDecl {
                    local: true,
                    signed,
                    range: range.clone(),
                    integer: false,
                    real: false,
                    name: name.clone(),
                    value,
                    line,
                }));
                names.push(name.clone());
                prev = Some(name);
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p("}")?;
            let mut t = base;
            t.enum_names = Some(Rc::new(names));
            return Ok(Some(t));
        }
        if let Some((kind, signed, iw)) = builtin(&s) {
            self.bump();
            let signed = self.signing(signed);
            let mut dims = Vec::new();
            if let Some(w) = iw {
                dims.push((num(w as i64 - 1), num(0)));
            }
            while self.is_p("[") {
                dims.push(self.range()?);
            }
            return Ok(Some(SvType { kind, signed, dims, fields: None, int_kind: iw.is_some(), enum_names: None, unpacked: vec![] }));
        }
        if let Some(t) = self.types.get(&s).cloned() {
            self.bump();
            let mut t = (*t).clone();
            t.signed = self.signing(t.signed);
            // dimensions after a type name are outer to the type's own
            let mut outer = Vec::new();
            while self.is_p("[") {
                outer.push(self.range()?);
            }
            if !outer.is_empty() {
                t.int_kind = false;
                outer.extend(t.dims);
                t.dims = outer;
            }
            return Ok(Some(t));
        }
        Ok(None)
    }

    /// `typedef type name;` (also `typedef enum ... name;`, forward `typedef name;`).
    pub(crate) fn typedef(&mut self) -> Result<()> {
        self.bump();
        let Some(t) = self.data_type()? else {
            // forward declaration: typedef [struct|enum] name;
            self.eat_kw("struct");
            self.eat_kw("union");
            self.eat_kw("enum");
            self.ident()?;
            return self.expect_p(";");
        };
        let name = self.ident()?;
        let mut t = t;
        while self.is_p("[") {
            if matches!(self.peek_at(1), Tok::Punct("]" | "$")) || matches!(self.peek_at(1), Tok::Ident(s) if s == "$") {
                return self.err("dynamic arrays and queues in typedefs are not supported");
            }
            t.unpacked.push(self.range()?);
        }
        self.expect_p(";")?;
        self.types.insert(name, Rc::new(t));
        Ok(())
    }

    /// A declaration head for a data type (after the direction): kind, sign, ranges. Records
    /// the type for the declarators that follow.
    pub(crate) fn typed_decl_head(&mut self, dir: Option<Dir>, line: Line) -> Result<Decl> {
        let t = self.data_type()?.expect("caller checked for a type");
        let (kind, signed, range, packed) = t.decl_parts();
        self.decl_type = Some(Rc::new(t));
        Ok(Decl {
            dir,
            kind,
            net: NetType::Wire,
            explicit_kind: true,
            real_net: false,
            strength: None,
            delay: None,
            signed,
            range,
            packed,
            names: vec![],
            line,
        })
    }

    /// Note the type of a declared name (struct members are resolved through it).
    pub(crate) fn declare_var(&mut self, name: &str, unpacked: &[Range]) {
        match &self.decl_type {
            Some(t) => {
                self.vars.insert(name.to_string(), (t.clone(), unpacked.to_vec()));
            }
            None => {
                self.vars.remove(name);
            }
        }
    }

    /// A reference to a variable with a struct type, from its name: indexes, members and
    /// selects become one `[off +: w]` select of the flat vector. Records the selected type
    /// (`self.last_type`) for an assignment pattern on the other side.
    pub(crate) fn struct_access(&mut self, name: String, lvalue: bool) -> Result<Expr> {
        let (t, unpacked) = self.vars[&name].clone();
        let mut base = Expr::Ident(name);
        for _ in 0..unpacked.len() {
            if !self.is_p("[") {
                self.last_type = None;
                return Ok(base);
            }
            self.bump();
            let i = self.expr()?;
            self.expect_p("]")?;
            base = Expr::Index(Box::new(base), Box::new(i));
        }
        let mut cur: SvType = (*t).clone();
        let mut di = 0;
        let mut off = num(0);
        let mut width = cur.width();
        let mut touched = false;
        let mut whole = true; // the selection is a whole (sub)type, not a part of it
        loop {
            if self.is_p("[") && di < cur.dims.len() {
                self.bump();
                let r = cur.dims[di].clone();
                let elem = cur.width_from(di + 1);
                let a = self.expr()?;
                if self.eat_p(":") {
                    let b = self.expr()?;
                    self.expect_p("]")?;
                    off = add(off, mul(dim_pos(&r, b.clone()), elem.clone()));
                    width = mul(dim_size(&(a, b)), elem);
                    whole = false;
                } else if self.is_p("+:") || self.is_p("-:") {
                    let up = self.eat_p("+:");
                    if !up {
                        self.bump();
                    }
                    let n = self.expr()?;
                    self.expect_p("]")?;
                    // the element on the lsb side of the selected ones: [a +: n] is a..a+n-1,
                    // [a -: n] is a-n+1..a; the lower index is on the lsb side of [7:0]
                    let low = match (up, descending(&r)) {
                        (true, true) | (false, false) => a,
                        (true, false) => sub(add(a, n.clone()), num(1)),
                        (false, true) => add(sub(a, n.clone()), num(1)),
                    };
                    off = add(off, mul(dim_pos(&r, low), elem.clone()));
                    width = mul(n, elem);
                    whole = false;
                } else {
                    self.expect_p("]")?;
                    off = add(off, mul(dim_pos(&r, a), elem.clone()));
                    width = elem;
                    di += 1;
                    touched = true;
                    continue;
                }
                touched = true;
                break;
            }
            if self.is_p(".") && di == cur.dims.len() && cur.fields.is_some() {
                let Tok::Ident(m) = self.peek_at(1).clone() else { break };
                let st = cur.fields.clone().unwrap();
                let Some(k) = st.fields.iter().position(|f| f.0 == m) else {
                    return self.err(format!("no member '{m}' in the struct"));
                };
                self.bump();
                self.bump();
                if !st.union {
                    for f in &st.fields[k + 1..] {
                        off = add(off, f.1.width());
                    }
                }
                cur = st.fields[k].1.clone();
                di = 0;
                width = cur.width();
                touched = true;
                continue;
            }
            break;
        }
        self.last_type = if whole {
            let mut rest = cur.clone();
            rest.dims.drain(..di);
            if di > 0 {
                rest.int_kind = false;
            }
            Some(rest)
        } else {
            None
        };
        if !touched {
            return Ok(base);
        }
        let e = Expr::IndexedRange(Box::new(base), Box::new(off), Box::new(width), true);
        // a whole signed member reads as signed
        if !lvalue && whole && cur.signed && di == 0 {
            return Ok(Expr::SysCall("$signed".into(), vec![e]));
        }
        Ok(e)
    }

    /// `v.first()`, `.last()`, `.num()`, `.next(N)`, `.prev(N)`, `.name()` on a variable with an
    /// enum type, at the cursor (after the name).
    pub(crate) fn is_enum_method(&self, name: &str) -> bool {
        self.vars.get(name).is_some_and(|v| v.0.enum_names.is_some())
            && self.is_p(".")
            && matches!(self.peek_at(1), Tok::Ident(m) if matches!(m.as_str(), "first" | "last" | "num" | "next" | "prev" | "name"))
            && matches!(self.peek_at(2), Tok::Punct("("))
    }

    pub(crate) fn enum_method(&mut self, name: String) -> Result<Expr> {
        let names = self.vars[&name].0.enum_names.clone().unwrap();
        self.bump();
        let Tok::Ident(m) = self.bump() else { unreachable!() };
        self.expect_p("(")?;
        let arg = if self.is_p(")") { None } else { Some(self.expr()?) };
        self.expect_p(")")?;
        let n = names.len();
        let v = Expr::Ident(name);
        // a mux over the constants; a value that is none of them (or has x bits) gives `dflt`
        let chain = |val: &dyn Fn(usize) -> Expr, dflt: Expr| {
            (0..n).rev().fold(dflt, |acc, i| {
                let c = Expr::Binary(BinOp::CaseEq, Box::new(v.clone()), Box::new(Expr::Ident(names[i].clone())));
                Expr::Cond(Box::new(c), Box::new(val(i)), Box::new(acc))
            })
        };
        let x = Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: "x".into(), fill: true });
        Ok(match m.as_str() {
            "first" => Expr::Ident(names[0].clone()),
            "last" => Expr::Ident(names[n - 1].clone()),
            "num" => num(n as i64),
            "next" | "prev" => {
                let k = match &arg {
                    None => 1,
                    Some(e) => match lit(e) {
                        Some(k) if k >= 0 => k as usize,
                        _ => return self.err(format!(".{m}() needs a constant count")),
                    },
                };
                let step = if m == "next" { k % n } else { n - k % n };
                // the enum's width and signedness (the x default would make it unsigned)
                let t = self.vars[match &v { Expr::Ident(n) => n, _ => unreachable!() }].0.clone();
                cast(t.width(), chain(&|i| Expr::Ident(names[(i + step) % n].clone()), x), u64::from(t.signed))
            }
            _ => Expr::SysCall("$__str".into(), vec![chain(&|i| Expr::Str(names[i].clone()), Expr::Str(String::new()))]),
        })
    }

    /// `$left(v)`, `$size(v, 2)`, ... of a declared name (after the function name).
    pub(crate) fn is_array_query(&self, f: &str) -> bool {
        matches!(f, "$left" | "$right" | "$low" | "$high" | "$size" | "$increment" | "$dimensions" | "$unpacked_dimensions")
            && self.is_p("(")
            && matches!(self.peek_at(1), Tok::Ident(n) if self.vars.contains_key(n))
            && matches!(self.peek_at(2), Tok::Punct(")" | ","))
    }

    /// The value of an array query function: unpacked dimensions are numbered first, then the
    /// packed ones; a missing dimension gives x.
    pub(crate) fn array_query(&mut self, f: &str) -> Result<Expr> {
        self.expect_p("(")?;
        let name = self.ident()?;
        let k = if self.eat_p(",") {
            let e = self.expr()?;
            match lit(&e) {
                Some(k) => k,
                None => return self.err(format!("{f}: the dimension must be a constant")),
            }
        } else {
            1
        };
        self.expect_p(")")?;
        let (t, unpacked) = self.vars[&name].clone();
        let mut dims = unpacked.clone();
        dims.extend(t.dims.iter().cloned());
        if t.has_struct() {
            dims.push((sub(t.leaf_width(), num(1)), num(0)));
        }
        let x = Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: "x".into(), fill: true });
        match f {
            "$dimensions" => return Ok(num(dims.len() as i64)),
            "$unpacked_dimensions" => return Ok(num(unpacked.len() as i64)),
            _ => {}
        }
        let Some((l, r)) = (k >= 1).then(|| dims.get(k as usize - 1)).flatten().cloned() else { return Ok(x) };
        let cmp = |op: BinOp, a: &Expr, b: &Expr, t: Expr, e: Expr| match (lit(a), lit(b)) {
            (Some(x), Some(y)) => {
                let c = match op {
                    BinOp::Ge => x >= y,
                    _ => x <= y,
                };
                if c { t } else { e }
            }
            _ => Expr::Cond(Box::new(Expr::Binary(op, Box::new(a.clone()), Box::new(b.clone()))), Box::new(t), Box::new(e)),
        };
        Ok(match f {
            "$left" => l,
            "$right" => r,
            "$low" => cmp(BinOp::Le, &l, &r, l.clone(), r.clone()),
            "$high" => cmp(BinOp::Ge, &l, &r, l.clone(), r.clone()),
            "$size" => dim_size(&(l, r)),
            _ => cmp(BinOp::Ge, &l, &r, num(1), num(-1)),
        })
    }

    /// `'{...}` after the `'`: positional values, `name: value`, `default: value`.
    pub(crate) fn pattern(&mut self) -> Result<Expr> {
        self.expect_p("{")?;
        let mut items = Vec::new();
        if !self.is_p("}") {
            loop {
                let key = match (self.peek().clone(), self.peek_at(1).clone()) {
                    (Tok::Ident(k), Tok::Punct(":")) => {
                        self.bump();
                        self.bump();
                        Some(k)
                    }
                    _ => None,
                };
                let v = self.expr()?;
                items.push(match key {
                    Some(k) => Expr::SysCall("$__named".into(), vec![Expr::Str(k), v]),
                    None => v,
                });
                if !self.eat_p(",") {
                    break;
                }
            }
        }
        self.expect_p("}")?;
        Ok(Expr::SysCall("$__pattern".into(), items))
    }

    /// Replace assignment patterns in `e` (at its top, or in a conditional's branches) by
    /// concatenations for type `t`.
    pub(crate) fn resolve_pattern(&self, e: Expr, t: &SvType) -> Result<Expr> {
        match e {
            Expr::SysCall(n, items) if n == "$__pattern" => self.build_pattern(items, t),
            Expr::Cond(c, a, b) => Ok(Expr::Cond(c, Box::new(self.resolve_pattern(*a, t)?), Box::new(self.resolve_pattern(*b, t)?))),
            e => Ok(e),
        }
    }

    fn build_pattern(&self, items: Vec<Expr>, t: &SvType) -> Result<Expr> {
        let mut pos = Vec::new();
        let mut named = Vec::new();
        let mut default = None;
        for it in items {
            match it {
                Expr::SysCall(n, mut kv) if n == "$__named" => {
                    let v = kv.pop().unwrap();
                    let Some(Expr::Str(k)) = kv.pop() else { unreachable!() };
                    if k == "default" {
                        default = Some(v);
                    } else {
                        named.push((k, v));
                    }
                }
                v => pos.push(v),
            }
        }
        // the parts: members of a struct, or the elements of the outermost dimension
        let parts: Vec<(Option<String>, SvType)> = if !t.dims.is_empty() && !t.int_kind {
            let r = &t.dims[0];
            let n = match (lit(&r.0), lit(&r.1)) {
                (Some(a), Some(b)) => (a - b).unsigned_abs() as usize + 1,
                _ => return self.err("an assignment pattern needs constant dimensions"),
            };
            let mut el = t.clone();
            el.dims.remove(0);
            (0..n).map(|_| (None, el.clone())).collect()
        } else if let Some(s) = &t.fields {
            if s.union {
                return self.err("assignment patterns for unions are not supported");
            }
            s.fields.iter().map(|(n, ft)| (Some(n.clone()), ft.clone())).collect()
        } else {
            // a vector: '{default: v} fills it, else the values are its bits
            return match default {
                Some(v) => Ok(cast(t.width(), v, 0)),
                None => Ok(Expr::Concat(pos.into_iter().map(|v| cast(num(1), v, 0)).collect())),
            };
        };
        if !pos.is_empty() && pos.len() != parts.len() {
            return self.err(format!("assignment pattern has {} values for {} elements", pos.len(), parts.len()));
        }
        let mut out = Vec::new();
        for (k, (name, pt)) in parts.iter().enumerate() {
            let v = if let Some(v) = pos.get(k) {
                v.clone()
            } else if let Some((_, v)) = named.iter().find(|(n, _)| Some(n) == name.as_ref()) {
                v.clone()
            } else if let Some(v) = &default {
                v.clone()
            } else {
                return self.err(format!("assignment pattern has no value for member {}", name.clone().unwrap_or_default()));
            };
            let v = self.resolve_pattern(v, pt)?;
            out.push(match v {
                Expr::Concat(_) => v,
                v => cast(pt.width(), v, 0),
            });
        }
        Ok(Expr::Concat(out))
    }

    /// `T'(e)` with the type already parsed.
    pub(crate) fn cast_to(&mut self, t: &SvType) -> Result<Expr> {
        self.expect_p("'")?;
        self.expect_p("(")?;
        let e = self.expr()?;
        self.expect_p(")")?;
        let e = self.resolve_pattern(e, t)?;
        Ok(cast(t.width(), e, if t.signed { 1 } else { 0 }))
    }

    /// The width of a type name or type keyword at the cursor (for `$bits(T)`), consuming it.
    pub(crate) fn type_width_here(&mut self) -> Result<Option<Expr>> {
        if self.is_pkg_type() {
            return Ok(self.data_type()?.map(|t| t.bits()));
        }
        let is_type = match self.peek() {
            Tok::Ident(s) => self.types.contains_key(s) || builtin(s).is_some() || s == "struct" || s == "enum",
            _ => false,
        };
        if !is_type || !matches!(self.peek_at(1), Tok::Punct(")" | "," | "[")) && !matches!(self.peek(), Tok::Ident(s) if s == "struct" || s == "enum") {
            return Ok(None);
        }
        Ok(self.data_type()?.map(|t| t.bits()))
    }
}

/// `w - 1`, folded for literals.
pub fn minus_one(w: Expr) -> Expr {
    sub(w, num(1))
}

/// `$__cast(width, sign, e)`: sign 0 unsigned, 1 signed, 2 the expression's.
pub fn cast(width: Expr, e: Expr, sign: u64) -> Expr {
    Expr::SysCall("$__cast".into(), vec![width, Expr::Num(dec(sign)), e])
}
