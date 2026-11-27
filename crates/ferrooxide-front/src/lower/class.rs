//! Classes: properties, methods, `this` and `super`.
//!
//! Inside a method, a property or method of the class (or of a base class) named without an
//! object is reached through `this`: after lowering a method body, such names become
//! `this.p` / `this.m(...)` unless a local declaration (an argument, a variable, a block
//! variable) hides them.
//!
//! A static property is one variable `C$p` next to the class (in the module, package or
//! `$unit` declaring it); `p` in a method, `obj.p` and `C::p` all name it.

use super::{Lower, SvType};
use crate::ast::*;
use crate::syntax::kw::Kw;
use crate::syntax::tree as t;
use crate::Result;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The names a class's methods reach through `this` (with those of its base classes).
#[derive(Debug, Default, Clone)]
pub(crate) struct ClassNames {
    /// the class
    pub name: String,
    pub props: HashSet<String>,
    pub methods: HashSet<String>,
    /// static methods (also of the base classes): called without `this`
    pub static_methods: HashSet<String>,
    /// static properties: name -> the variable holding it
    pub statics: HashMap<String, String>,
    /// static properties of a parameterized class (one per specialization, `C #(..)::x`)
    pub spec_statics: HashSet<String>,
    /// typedefs declared in the class (`C::T`)
    pub types: HashMap<String, Rc<SvType>>,
    /// parameters and enum constants declared in the class (`C::P`, `obj.P`)
    pub consts: HashSet<String>,
    /// type parameters of the class and its base classes (types in a derived class too)
    pub type_params: HashMap<String, Rc<SvType>>,
    /// a class declared later (`typedef class C;`) or a specialization of one: its names
    /// are not known yet
    pub later: bool,
    /// a parameterized class (not a specialization of one): outside the class, `C::x`
    /// needs parameter values (`C #(..)::x`)
    pub parameterized: bool,
    /// an interface class
    pub interface: bool,
    /// the pure constraints (IEEE 1800-2017 18.5.2) of the class and its base classes that
    /// no class overrides yet
    pub pure_constraints: HashSet<String>,
    /// the types, parameters and constants of the class (also inherited ones) -> the class
    /// declaring them (name conflicts of interface classes, IEEE 1800-2017 8.26.6)
    pub origins: HashMap<String, String>,
}

impl Lower {
    /// A name class `class` declares: an error when it declares it already.
    fn own_name(&self, own: &mut HashSet<String>, n: &str, line: Line, class: &str) -> Result<()> {
        if !own.insert(n.to_string()) {
            return Err(crate::Error { msg: format!("'{n}' is already declared in class '{class}'"), file: self.file.clone(), line });
        }
        Ok(())
    }

    /// `class name [#(params)] [extends base]; items endclass`
    /// `rest`: the items after it in its scope, which hold the bodies of its extern methods.
    pub(crate) fn class_decl(&mut self, c: &t::ClassDecl, rest: &[&t::Item]) -> Result<ClassDecl> {
        let saved_specs = std::mem::take(&mut self.specs);
        let mut bodies = HashMap::new();
        let mut cbodies = HashMap::new();
        for it in rest {
            if let t::Item::Subroutine(s) = it {
                if let [sc] = s.name.scopes.as_slice() {
                    if sc.name == c.name && sc.params.is_none() {
                        bodies.entry(s.name.name.clone()).or_insert_with(|| (**s).clone());
                    }
                }
            }
            // the bodies of constraint prototypes (IEEE 1800-2017 18.5.1)
            if let t::Item::Constraint(k) = it {
                if let [sc] = k.name.scopes.as_slice() {
                    if sc.name == c.name && sc.params.is_none() {
                        cbodies.entry(k.name.name.clone()).or_insert_with(|| (**k).clone());
                    }
                }
            }
        }
        let saved_cbodies = std::mem::replace(&mut self.extern_cbodies, cbodies);
        // (the bodies are lowered here: classes declared before the last of them are known)
        let is_body = |it: &&t::Item| matches!(it, t::Item::Subroutine(s) if matches!(s.name.scopes.as_slice(), [sc] if sc.name == c.name && sc.params.is_none()));
        // (and the typedefs before it, when a body names them)
        let mut later = HashMap::new();
        if let Some(last) = rest.iter().rposition(is_body) {
            for it in &rest[..last] {
                if let t::Item::Class(x) = it {
                    if !self.types.contains_key(&x.name) {
                        self.types.insert(x.name.clone(), Self::class_type(&x.name));
                    }
                }
                if let t::Item::Typedef(td @ t::Typedef::Type { name, .. }) = it {
                    if !self.types.contains_key(name) {
                        later.entry(name.clone()).or_insert_with(|| td.clone());
                    }
                }
            }
        }
        let saved_later = std::mem::replace(&mut self.later_typedefs, later);
        let saved_bodies = std::mem::replace(&mut self.extern_bodies, bodies);
        let r = self.class_decl_inner(c);
        self.extern_bodies = saved_bodies;
        self.later_typedefs = saved_later;
        self.extern_cbodies = saved_cbodies;
        let specs = std::mem::replace(&mut self.specs, saved_specs);
        r.map(|mut d| {
            d.specs = specs;
            d
        })
    }

    /// `n` with the parameter values of a parameterized class (`C #(..)`, `C #(..)::x`,
    /// `P::C #(..)`) replaced by the name of a new specialization (`C#1`, see
    /// `Item::ClassSpec`); `n` itself where it has none.
    pub(crate) fn despec(&mut self, n: &t::Name, loc: t::Loc) -> Result<t::Name> {
        if let Some(args) = &n.params {
            if n.scopes.iter().any(|s| s.params.is_some()) || n.scopes.len() > 1 {
                return self.unsupported(loc, "this parameterized class name");
            }
            let (full, names) = match n.scopes.first() {
                None => match self.classes.get(&n.name) {
                    Some(c) => (n.name.clone(), c.clone()),
                    // a class declared later (`typedef class C;`): its names come with it
                    None if self.types.get(&n.name).is_some_and(|t| t.class.is_some()) => (n.name.clone(), Rc::new(ClassNames { later: true, ..Default::default() })),
                    None => return self.err(loc, format!("'{}' is not a class", n.name)),
                },
                Some(p) => match self.packages.get(&p.name).and_then(|k| k.classes.get(&n.name)).cloned() {
                    Some(c) => {
                        self.use_package(&p.name);
                        (format!("{}::{}", p.name, n.name), c)
                    }
                    None => return self.err(loc, format!("'{}::{}' is not a class", p.name, n.name)),
                },
            };
            let spec = self.new_spec(&full, &n.name, names, args, loc)?;
            return Ok(t::Name { scopes: vec![], name: spec, params: None, loc: n.loc });
        }
        // `P::C #(..)::x`: of a specialization of package P's class C
        if let [p, s, more @ ..] = n.scopes.as_slice() {
            if let (None, Some(args), true) = (&p.params, &s.params, more.iter().all(|m| m.params.is_none())) {
                if let Some(names) = self.packages.get(&p.name).and_then(|k| k.classes.get(&s.name)).cloned() {
                    self.use_package(&p.name);
                    let spec = self.new_spec(&format!("{}::{}", p.name, s.name), &s.name, names, args, loc)?;
                    let mut scopes = vec![t::NamePart { name: spec, params: None }];
                    scopes.extend(more.iter().cloned());
                    return Ok(t::Name { scopes, name: n.name.clone(), params: None, loc: n.loc });
                }
            }
        }
        // `C #(..)::x`, `C #(..)::T::x`
        if let Some((s, more)) = n.scopes.split_first() {
            if let (Some(args), true) = (&s.params, more.iter().all(|m| m.params.is_none())) {
                let names = match self.classes.get(&s.name) {
                    Some(c) => c.clone(),
                    // a class declared later (`typedef class C;`): its names come with it
                    None if self.types.get(&s.name).is_some_and(|t| t.class.is_some()) => Rc::new(ClassNames { later: true, ..Default::default() }),
                    None => return self.err(loc, format!("'{}' is not a class", s.name)),
                };
                let spec = self.new_spec(&s.name, &s.name, names, args, loc)?;
                let mut scopes = vec![t::NamePart { name: spec, params: None }];
                scopes.extend(more.iter().cloned());
                return Ok(t::Name { scopes, name: n.name.clone(), params: None, loc: n.loc });
            }
        }
        if n.scopes.iter().any(|s| s.params.is_some()) {
            return self.unsupported(loc, "this parameterized class name");
        }
        Ok(n.clone())
    }

    /// `n` with leading scopes `C::T` (T a typedef in class C that names a class D) replaced
    /// by D, as often as they occur.
    pub(crate) fn class_typedef_scopes(&self, n: &t::Name) -> t::Name {
        let mut n = n.clone();
        while n.scopes.len() > 1 && n.scopes[..2].iter().all(|s| s.params.is_none()) {
            let Some(d) = self.classes.get(&n.scopes[0].name).and_then(|c| c.types.get(&n.scopes[1].name)).and_then(|t| t.class.clone()) else { break };
            n.scopes.splice(0..2, [t::NamePart { name: d, params: None }]);
        }
        n
    }

    /// A new specialization of class `full` (named `base` without its package) with
    /// parameter values `args`: its name, known as a class with `names` from here on.
    fn new_spec(&mut self, full: &str, base: &str, names: Rc<ClassNames>, args: &[t::ParamArg], loc: t::Loc) -> Result<String> {
        let mut conns = Vec::new();
        for p in args {
            conns.push(match p {
                t::ParamArg::Positional(e) => Conn::Positional(Some(self.expr(e)?)),
                t::ParamArg::Empty => Conn::Positional(None),
                t::ParamArg::Named(n, e) => Conn::Named(
                    n.clone(),
                    match e {
                        Some(e) => Some(self.expr(e)?),
                        None => None,
                    },
                ),
            });
        }
        self.spec_count += 1;
        let spec = format!("{base}#{}", self.spec_count);
        let mut names = (*names).clone();
        names.parameterized = false;
        self.classes.insert(spec.clone(), Rc::new(names));
        self.spec_of.push((spec.clone(), full.to_string()));
        self.types.insert(spec.clone(), Self::class_type(&spec));
        self.specs.push(Item::ClassSpec { name: spec.clone(), class: full.to_string(), args: conns, line: loc.line });
        Ok(spec)
    }

    fn class_decl_inner(&mut self, c: &t::ClassDecl) -> Result<ClassDecl> {
        let saved = self.in_class.replace(c.name.clone());
        let r = self.class_decl_body(c);
        self.in_class = saved;
        r
    }

    /// `C::x` of a parameterized class C needs parameter values outside C (IEEE 1800-2017
    /// 8.25.1).
    pub(crate) fn check_unspecialized(&self, class: &str, loc: t::Loc) -> Result<()> {
        if self.classes.get(class).is_some_and(|c| c.parameterized) && self.in_class.as_deref() != Some(class) {
            return self.err(loc, format!("'{class}' is a parameterized class: '{class}::' needs parameter values here ('{class} #(...)::')"));
        }
        Ok(())
    }

    fn class_decl_body(&mut self, c: &t::ClassDecl) -> Result<ClassDecl> {
        let line = c.loc.line;
        let name = self.name(&c.name, c.loc);
        let mut names = ClassNames { name: name.clone(), interface: c.interface, ..Default::default() };
        // the class is a type from here on (also inside itself)
        self.types.insert(name.clone(), Self::class_type(&name));
        // (the class's parameters and typedefs, and those of its base class, stay inside)
        let saved_types = self.types.clone();
        // the parameter ports: constants of the class (`C::P`), type parameters
        let mut params = Vec::new();
        let mut own: HashSet<String> = HashSet::new();
        names.parameterized = c.params.is_some();
        if let Some(ps) = &c.params {
            self.param_ports(ps, &mut params)?;
            for p in &params {
                match p {
                    Item::Param(p) => {
                        self.own_name(&mut own, &p.name, p.line, &name)?;
                        names.consts.insert(p.name.clone());
                    }
                    Item::TypeParam { name: n, line, .. } => {
                        self.own_name(&mut own, n, *line, &name)?;
                        if let Some(t) = self.types.get(n) {
                            names.type_params.insert(n.clone(), t.clone());
                        }
                    }
                    _ => {}
                }
                if let Item::Param(ParamDecl { name: n, .. }) | Item::TypeParam { name: n, .. } = p {
                    names.origins.insert(n.clone(), name.clone());
                }
            }
        }
        // the interface classes it implements; those an interface class extends
        // (IEEE 1800-2017 8.26)
        let mut iface_refs: Vec<&t::Name> = Vec::new();
        if c.interface {
            iface_refs.extend(c.extends.iter().map(|e| &e.0));
            if c.extends.as_ref().is_some_and(|e| e.1.is_some()) {
                return self.err(c.loc, "an interface class takes no constructor arguments");
            }
        }
        iface_refs.extend(c.implements.iter());
        let mut interfaces = Vec::new();
        // (a type, parameter or constant name -> the class declaring it, from one interface)
        let mut inherited: HashMap<String, (String, String)> = HashMap::new();
        let mut conflicts: Vec<(String, String, String)> = Vec::new();
        for n in iface_refs {
            let (b, bname) = self.class_ref(n, c.loc, "interface class")?;
            if !b.interface {
                let what = if c.interface { "extends" } else { "implements" };
                return self.err(c.loc, format!("class '{name}' {what} '{}', which is not an interface class", n.name));
            }
            if c.interface {
                names.methods.extend(b.methods.iter().cloned());
                for (k, o) in &b.origins {
                    match inherited.get(k) {
                        Some((o2, from)) if o2 != o => conflicts.push((k.clone(), from.clone(), bname.clone())),
                        _ => {
                            inherited.insert(k.clone(), (o.clone(), bname.clone()));
                        }
                    }
                }
                names.types.extend(b.types.iter().map(|(k, v)| (k.clone(), v.clone())));
                names.consts.extend(b.consts.iter().cloned());
            } else {
                // (an abstract class may leave methods to a derived class)
                names.methods.extend(b.methods.iter().cloned());
            }
            interfaces.push(bname);
        }
        for (k, (o, _)) in inherited {
            names.origins.entry(k).or_insert(o);
        }
        // (after the parameter ports: `extends B #(T)` may use them)
        let (base, base_args) = match c.extends.as_ref().filter(|_| !c.interface) {
            None => (None, None),
            Some((n, args)) => {
                let (b, bname) = self.class_ref(n, c.loc, "base class")?;
                if b.interface {
                    return self.err(c.loc, format!("class '{name}' extends interface class '{}' (a class implements it)", n.name));
                }
                names.origins.extend(b.origins.iter().map(|(k, v)| (k.clone(), v.clone())));
                names.pure_constraints.extend(b.pure_constraints.iter().cloned());
                names.props.extend(b.props.iter().cloned());
                names.methods.extend(b.methods.iter().cloned());
                names.static_methods.extend(b.static_methods.iter().cloned());
                names.statics.extend(b.statics.iter().map(|(k, v)| (k.clone(), v.clone())));
                names.spec_statics.extend(b.spec_statics.iter().cloned());
                names.types.extend(b.types.iter().map(|(k, v)| (k.clone(), v.clone())));
                names.consts.extend(b.consts.iter().cloned());
                // (the base's type parameters, unless the class has its own of that name)
                for (k, v) in &b.type_params {
                    if !names.type_params.contains_key(k) {
                        names.type_params.insert(k.clone(), v.clone());
                        self.types.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
                let args = match args {
                    Some(a) => Some(self.call_args(a)?),
                    None => None,
                };
                (Some(bname), args)
            }
        };
        for (k, v) in &names.types {
            self.types.insert(k.clone(), v.clone());
        }
        // properties are names in the methods (their types for member chains)
        let saved_vars = self.vars.clone();
        let mut props = Vec::new();
        let mut statics = Vec::new();
        let mut consts = Vec::new();
        let mut rands: Vec<(String, bool)> = Vec::new();
        let mut static_props = Vec::new();
        let mut hidden: Vec<(String, bool)> = Vec::new();
        // (the names the class itself declares, one item each, IEEE 1800-2017 8.3: `own`)
        let vis = |quals: &[Kw]| {
            if quals.contains(&Kw::Local) {
                Some(true)
            } else if quals.contains(&Kw::Protected) {
                Some(false)
            } else {
                None
            }
        };
        let items: Vec<&t::Item> = c.items.iter().filter_map(|i| if let t::ClassItem::Item(x) = i { Some(x) } else { None }).collect();
        for it in &c.items {
            // parameters and typedefs (enum constants are parameters) in source order with
            // the properties that may use them
            if let t::ClassItem::Item(i @ (t::Item::Param(_) | t::Item::Typedef(_))) = it {
                let before = self.types.clone();
                let mut v = Vec::new();
                // constants in the class's types refer to the class's parameters (`C::W`)
                let known: HashMap<String, String> = consts.iter().map(|p: &ParamDecl| p.name.clone()).chain(names.consts.iter().cloned()).map(|n| (n, name.clone())).collect();
                let saved_scope = self.type_scope.replace(known);
                let r = match i {
                    // a type parameter of the class body: nothing overrides it (a class is
                    // specialized through its parameter port list), so it names its type as
                    // a typedef does (IEEE 1800-2017 8.25)
                    t::Item::Param(t::ParamDecl { kind: t::ParamKind::Type { assigns }, .. }) => (|| {
                        for (n, d, loc) in assigns {
                            let Some(d) = d else { return self.err(*loc, format!("type parameter '{n}' of a class body needs a type")) };
                            let td = t::Typedef::Type { ty: d.clone(), name: n.clone(), dims: vec![], loc: *loc };
                            self.typedef(&td, &items)?;
                            v.append(&mut self.pending);
                        }
                        Ok(())
                    })(),
                    _ => self.module_item(i, &items, &mut v),
                };
                self.type_scope = saved_scope;
                r?;
                for x in v.into_iter().chain(std::mem::take(&mut self.pending)) {
                    match x {
                        Item::Param(p) => {
                            self.own_name(&mut own, &p.name, p.line, &name)?;
                            names.origins.insert(p.name.clone(), name.clone());
                            consts.push(p)
                        }
                        _ => return self.unsupported(super::item_loc(i), &format!("{} in a class", super::item_what(i))),
                    }
                }
                if let t::Item::Typedef(t::Typedef::Type { name: n, loc, .. }) = i {
                    self.own_name(&mut own, n, loc.line, &name)?;
                    names.origins.insert(n.clone(), name.clone());
                }
                if let t::Item::Param(t::ParamDecl { kind: t::ParamKind::Type { assigns }, .. }) = i {
                    for (n, _, loc) in assigns {
                        self.own_name(&mut own, n, loc.line, &name)?;
                        names.origins.insert(n.clone(), name.clone());
                    }
                }
                for (k, t) in &self.types {
                    if before.get(k).is_none_or(|o| !Rc::ptr_eq(o, t)) {
                        names.types.insert(k.clone(), t.clone());
                    }
                }
                continue;
            }
            let t::ClassItem::Property { quals, decl } = it else { continue };
            if c.interface {
                return self.err(super::item_loc(decl), format!("interface class '{name}' cannot have properties"));
            }
            let is_static = quals.contains(&Kw::Static);
            let rand = quals.iter().find(|q| matches!(q, Kw::Rand | Kw::Randc)).map(|q| *q == Kw::Randc);
            let t::Item::Var(v) = decl else {
                return self.unsupported(super::item_loc(decl), &format!("{} in a class", super::item_what(decl)));
            };
            if v.lifetime.is_some() {
                return self.unsupported(v.loc, "a class property lifetime");
            }
            let mut d = self.decl_head(None, None, v.var, &v.ty, None, None, v.loc)?;
            // an enum type declared here: its constants belong to the class
            for x in std::mem::take(&mut self.pending) {
                match x {
                    Item::Param(p) => {
                        self.own_name(&mut own, &p.name, p.line, &name)?;
                        consts.push(p)
                    }
                    _ => return self.unsupported(v.loc, "this property type"),
                }
            }
            d.names = self.declarators(&v.decls)?;
            for n in &d.names {
                self.own_name(&mut own, &n.name, n.line, &name)?;
            }
            if let Some(cyclic) = rand {
                if is_static && c.params.is_some() {
                    return self.unsupported(v.loc, "a static rand property of a parameterized class");
                }
                // (a static one by the name of its variable, `C$p`)
                let var = |n: &str| if is_static { format!("{name}${n}") } else { n.to_string() };
                rands.extend(d.names.iter().map(|n| (var(&n.name), cyclic)));
            }
            if is_static {
                if c.params.is_some() {
                    names.spec_statics.extend(d.names.iter().map(|n| n.name.clone()));
                    static_props.push(Item::Decl(d));
                    continue;
                }
                for n in &mut d.names {
                    let var = format!("{name}${}", n.name);
                    names.statics.insert(n.name.clone(), var.clone());
                    n.name = var;
                }
                statics.push(d);
                continue;
            }
            for n in &d.names {
                names.props.insert(n.name.clone());
                if let Some(l) = vis(quals) {
                    hidden.push((n.name.clone(), l));
                }
            }
            props.push(d);
        }
        // a name two interfaces give differently needs a declaration of the class's own
        if let Some((k, a, b)) = conflicts.iter().find(|x| !own.contains(&x.0)) {
            return self.err(c.loc, format!("'{k}' of interface classes '{a}' and '{b}' conflict; interface class '{name}' must declare its own '{k}'"));
        }
        let mut virtuals = Vec::new();
        let mut static_methods = Vec::new();
        let mut pures = Vec::new();
        for it in &c.items {
            if let t::ClassItem::Method(m) = it {
                let n = &m.name.name;
                if c.interface && !(m.quals.contains(&Kw::Pure) && m.quals.contains(&Kw::Virtual)) {
                    return self.err(m.loc, format!("method '{n}' of interface class '{name}' must be pure virtual"));
                }
                if m.quals.contains(&Kw::Pure) {
                    pures.push(n.clone());
                }
                self.own_name(&mut own, n, m.loc.line, &name)?;
                names.methods.insert(n.clone());
                if let Some(l) = vis(&m.quals) {
                    hidden.push((n.clone(), l));
                }
                // (a method of this class hides a static one of a base class)
                names.static_methods.remove(n);
                if m.quals.contains(&Kw::Virtual) {
                    virtuals.push(n.clone());
                }
                if m.quals.contains(&Kw::Static) {
                    if m.quals.contains(&Kw::Virtual) || n == "new" {
                        return self.err(m.loc, format!("method '{n}' cannot be static"));
                    }
                    names.static_methods.insert(n.clone());
                    static_methods.push(n.clone());
                }
            }
        }
        names.consts.extend(consts.iter().map(|p| p.name.clone()));
        // pure constraints: declared in a virtual class, overridden by a constraint of that
        // name in a derived class; a class that is not virtual has none left
        for it in &c.items {
            if let t::ClassItem::Constraint(k) = it {
                if k.quals.contains(&Kw::Pure) {
                    if !c.virtual_ {
                        return self.err(k.loc, format!("pure constraint '{}' must be in a virtual class", k.name.name));
                    }
                    names.pure_constraints.insert(k.name.name.clone());
                } else {
                    names.pure_constraints.remove(&k.name.name);
                }
            }
        }
        if !c.virtual_ {
            let mut left: Vec<&String> = names.pure_constraints.iter().collect();
            left.sort();
            if let Some(k) = left.first() {
                return self.err(c.loc, format!("class '{name}' does not override pure constraint '{k}'"));
            }
        }
        // (the class's names are known before its methods, which may call each other)
        let names = Rc::new(names);
        self.classes.insert(name.clone(), names.clone());
        // specializations made before (of a forward declaration) know its names now
        let mut sn = (*names).clone();
        sn.parameterized = false;
        let sn = Rc::new(sn);
        for (s, _) in self.spec_of.iter().filter(|(_, g)| *g == name) {
            self.classes.insert(s.clone(), sn.clone());
        }
        for d in props.iter_mut().chain(statics.iter_mut()) {
            for n in &mut d.names {
                if let Some(e) = &mut n.init {
                    this_expr(e, &names, &[]);
                }
            }
        }
        let mut methods = Vec::new();
        let mut constraints = Vec::new();
        // (constraint names are the class's names too)
        let mut own_c = own.clone();
        for it in &c.items {
            match it {
                t::ClassItem::Property { .. } => {}
                t::ClassItem::Method(m) => {
                    let merged;
                    let m = if m.quals.contains(&Kw::Extern) {
                        merged = self.extern_method(m, &name)?;
                        &merged
                    } else if m.quals.contains(&Kw::Pure) {
                        // a pure virtual method: never called (the class is abstract, a
                        // derived class overrides it); an empty body
                        if !m.quals.contains(&Kw::Virtual) || !(c.virtual_ || c.interface) {
                            return self.err(m.loc, format!("pure method '{}' must be virtual, in a virtual class", m.name.name));
                        }
                        let mut s = (**m).clone();
                        s.quals.retain(|q| *q != Kw::Pure);
                        s.body = Some((vec![], vec![]));
                        merged = Box::new(s);
                        &merged
                    } else {
                        m
                    };
                    if let Some(q) = m.quals.iter().find(|q| !matches!(q, Kw::Virtual | Kw::Local | Kw::Protected | Kw::Static)) {
                        return self.unsupported(m.loc, &format!("a {} method", q.as_str()));
                    }
                    // (IEEE 1800-2017 8.6: methods have automatic lifetime)
                    if m.lifetime == Some(t::Lifetime::Static) {
                        return self.err(m.loc, format!("method '{}' cannot have a static lifetime", m.name.name));
                    }
                    let mut f = self.subroutine_in(m, true)?;
                    f.automatic = true;
                    let mut locals: Vec<String> = vec![f.name.clone()];
                    for d in &f.decls {
                        locals.extend(d.names.iter().map(|n| n.name.clone()));
                    }
                    for d in &mut f.decls {
                        for n in &mut d.names {
                            if let Some(e) = &mut n.init {
                                this_expr(e, &names, &locals);
                            }
                        }
                    }
                    this_stmt(&mut f.body, &names, &mut locals);
                    methods.push(f);
                }
                t::ClassItem::Constraint(k) if c.interface => {
                    return self.err(k.loc, format!("interface class '{name}' cannot have constraints"));
                }
                t::ClassItem::Constraint(k) if k.quals.contains(&Kw::Pure) => {
                    if k.items.is_some() {
                        return self.err(k.loc, format!("pure constraint '{}' has no body", k.name.name));
                    }
                    self.own_name(&mut own_c, &k.name.name, k.loc.line, &name)?;
                }
                t::ClassItem::Constraint(k) => {
                    let ext = k.quals.contains(&Kw::Extern);
                    if ext && k.items.is_some() {
                        return self.err(k.loc, format!("extern constraint '{}' has its body outside the class", k.name.name));
                    }
                    // a prototype: the body declared after the class (IEEE 1800-2017 18.5.1);
                    // without one, an `extern` prototype is an error, an implicit one empty
                    let merged;
                    let k = if k.items.is_none() {
                        let n = &k.name.name;
                        let items = match self.extern_cbodies.get(n).cloned() {
                            Some(b) => {
                                if b.static_ != k.static_ {
                                    return self.err(b.loc, format!("'{name}::{n}' and its prototype differ in `static`"));
                                }
                                self.extern_done.insert((format!("{name}::{n}"), b.loc.line as usize));
                                b.items.unwrap_or_default()
                            }
                            None if ext => return self.err(k.loc, format!("extern constraint '{n}' of class '{name}' has no body ('{name}::{n}')")),
                            None => vec![],
                        };
                        merged = t::ConstraintDecl { quals: vec![], items: Some(items), ..(**k).clone() };
                        &merged
                    } else {
                        &**k
                    };
                    let mut k = self.constraint_decl(k)?;
                    self.own_name(&mut own_c, &k.name, k.line, &name)?;
                    for it in &mut k.items {
                        this_citem(it, &names, &mut Vec::new());
                    }
                    constraints.push(k);
                }
                t::ClassItem::Item(t::Item::Empty(_) | t::Item::Param(_) | t::Item::Typedef(_)) => {}
                t::ClassItem::Item(i) => return self.unsupported(super::item_loc(i), &format!("{} in a class", super::item_what(i))),
            }
        }
        // the static properties' variables, with their types, for the declaring scope
        let types: Vec<(String, Option<(Rc<SvType>, Vec<Range>)>)> =
            names.statics.iter().filter(|(_, v)| v.starts_with(&format!("{name}$"))).map(|(p, v)| (v.clone(), self.vars.get(p).cloned())).collect();
        self.vars = saved_vars;
        self.types = saved_types;
        for (v, t) in types {
            if let Some(t) = t {
                self.vars.insert(v, t);
            }
        }
        self.class_statics = statics.into_iter().map(Item::Decl).collect();
        let mut class_types: Vec<_> = names.types.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        class_types.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(ClassDecl {
            name,
            params,
            specs: vec![],
            base,
            base_args,
            interface: c.interface,
            interfaces,
            props,
            static_props,
            types: class_types,
            consts,
            methods,
            virtuals,
            pures,
            statics: static_methods,
            abstract_: c.virtual_ || c.interface,
            hidden,
            rand: rands,
            constraints,
            line,
        })
    }

    /// The class `n` names (`B`, `B #(..)`, `P::B`) as a base or interface class: its names
    /// and its name for the compiler.
    fn class_ref(&mut self, n: &t::Name, loc: t::Loc, what: &str) -> Result<(Rc<ClassNames>, String)> {
        // `B #(..)`: a specialization (in the class's scope)
        let n = &self.despec(n, loc)?;
        if n.scopes.len() > 1 {
            return self.unsupported(loc, &format!("this {what}"));
        }
        // `P::B`: a class of package P (named `P::B` for the compiler)
        match n.scopes.first() {
            Some(s) => {
                let Some(b) = self.packages.get(&s.name).and_then(|p| p.classes.get(&n.name)).cloned() else {
                    return self.err(loc, format!("unknown {what} '{}::{}'", s.name, n.name));
                };
                self.use_package(&s.name);
                Ok((b, format!("{}::{}", s.name, n.name)))
            }
            None => {
                let Some(b) = self.classes.get(&n.name).cloned().filter(|b| !(b.later && what == "interface class")) else {
                    // `extends T`, T a type parameter (IEEE 1800-2017 8.25): the class it
                    // names is known per specialization (in the compiler); names of its
                    // members need `this.` or `super.` here
                    if what == "base class" && n.params.is_none() && self.types.get(&n.name).is_some_and(|t| t.param.is_some()) {
                        return Ok((Rc::new(ClassNames { later: true, ..Default::default() }), n.name.clone()));
                    }
                    if self.types.get(&n.name).is_some_and(|t| t.class.is_some()) {
                        if what == "interface class" {
                            return self.err(loc, format!("interface class '{}' is used before its declaration", n.name));
                        }
                        return self.unsupported(loc, "a base class declared after the class");
                    }
                    return self.err(loc, format!("unknown {what} '{}'", n.name));
                };
                Ok((b, n.name.clone()))
            }
        }
    }

    /// An extern method (IEEE 1800-2017 8.24): the prototype `m` with its body from the
    /// `C::f` declaration after the class. Default argument values may be in either.
    fn extern_method(&mut self, m: &t::Subroutine, class: &str) -> Result<Box<t::Subroutine>> {
        let n = &m.name.name;
        let Some(b) = self.extern_bodies.get(n).cloned() else {
            return self.err(m.loc, format!("extern method '{n}' of class '{class}' has no body ('{class}::{n}')"));
        };
        if b.is_task != m.is_task {
            return self.err(b.loc, format!("'{class}::{n}' is declared as a {} in the class", if m.is_task { "task" } else { "function" }));
        }
        let pn = m.ports.as_ref().map_or(0, |p| p.len());
        let bn = b.ports.as_ref().map_or(0, |p| p.len());
        if pn != bn {
            return self.err(b.loc, format!("'{class}::{n}' has {bn} arguments, its prototype {pn}"));
        }
        self.extern_done.insert((format!("{class}::{n}"), b.loc.line as usize));
        let mut s = b;
        s.quals = m.quals.iter().copied().filter(|q| *q != Kw::Extern).collect();
        s.name = m.name.clone();
        if let (Some(ps), Some(bs)) = (&m.ports, &mut s.ports) {
            for (p, q) in ps.iter().zip(bs.iter_mut()) {
                if q.default.is_none() {
                    q.default = p.default.clone();
                }
            }
        }
        Ok(Box::new(s))
    }

    /// `constraint C::c { ... }` after class C: merged into it, the body of a prototype.
    pub(crate) fn constraint_body_done(&self, k: &t::ConstraintDecl) -> Result<()> {
        let (c, n) = (&k.name.scopes[0].name, &k.name.name);
        if !self.extern_done.contains(&(format!("{c}::{n}"), k.loc.line as usize)) {
            return self.err(k.loc, format!("constraint '{c}::{n}' has no prototype in class '{c}'"));
        }
        Ok(())
    }

    /// The type of handles of class `name`.
    pub(crate) fn class_type(name: &str) -> Rc<SvType> {
        let mut ct = SvType::vector(NetKind::Class, false, vec![(super::types::num(63), super::types::num(0))]);
        ct.int_kind = true;
        ct.class = Some(name.to_string());
        Rc::new(ct)
    }

    /// The type of a variable named `n` is a class.
    pub(crate) fn is_class_var(&self, n: &str) -> bool {
        n == "this" || n == "super" || self.vars.get(n).is_some_and(|v| v.0.class.is_some())
    }
}

impl Lower {
    /// The variable of static property `m` of the class of `e` (a class variable or `C`).
    pub(crate) fn static_prop(&self, e: &Expr, m: &str) -> Option<String> {
        let class = match &e.kind {
            ExprKind::Ident(v) => self.vars.get(v).and_then(|t| t.0.class.clone()).or_else(|| self.classes.contains_key(v).then(|| v.clone()))?,
            _ => return None,
        };
        self.classes.get(&class)?.statics.get(m).cloned()
    }
}

/// A static property of the class, unless a local hides it: its variable.
fn static_name<'c>(n: &str, c: &'c ClassNames, locals: &[String]) -> Option<&'c String> {
    c.statics.get(n).filter(|_| !locals.iter().any(|l| l == n))
}

/// A name of the class reached through `this`, unless a local hides it.
fn this_name(n: &str, c: &ClassNames, locals: &[String]) -> bool {
    c.props.contains(n) && !locals.iter().any(|l| l == n)
}

fn this() -> Box<Expr> {
    Box::new(ExprKind::Ident("this".into()).into())
}

/// `this.` before the class's names in an expression.
fn this_expr(e: &mut Expr, c: &ClassNames, locals: &[String]) {
    match &mut e.kind {
        ExprKind::Ident(n) if static_name(n, c, locals).is_some() => *e = ExprKind::Ident(static_name(n, c, locals).unwrap().clone()).into(),
        ExprKind::Hier(p) if static_name(&p[0], c, locals).is_some() => p[0] = static_name(&p[0], c, locals).unwrap().clone(),
        ExprKind::Member(o, n) if matches!(&o.kind, ExprKind::Ident(t) if t == "this") && c.statics.contains_key(n.as_str()) => {
            *e = ExprKind::Ident(c.statics[n.as_str()].clone()).into()
        }
        ExprKind::Ident(n) if this_name(n, c, locals) => *e = ExprKind::Member(this(), n.clone()).into(),
        ExprKind::Hier(p) if this_name(&p[0], c, locals) => {
            let mut x = ExprKind::Member(this(), p[0].clone());
            for m in &p[1..] {
                x = ExprKind::Member(Box::new(x.into()), m.clone());
            }
            *e = x.into();
        }
        ExprKind::Call(n, args) => {
            args.iter_mut().for_each(|a| this_expr(a, c, locals));
            if let Some(m) = static_member_call(n, c, locals) {
                *n = m;
            } else if static_call(n, c, locals) {
                *e = ExprKind::StaticCall(c.name.clone(), n.clone(), std::mem::take(args)).into();
            } else if let Some(x) = this_call(n, c, locals) {
                *e = ExprKind::MethodCall(Box::new(x.0), x.1, std::mem::take(args)).into();
            }
        }
        ExprKind::Index(a, b) => {
            this_expr(a, c, locals);
            this_expr(b, c, locals);
        }
        ExprKind::Range(a, b, d) | ExprKind::IndexedRange(a, b, d, _) | ExprKind::Cond(a, b, d) => {
            this_expr(a, c, locals);
            this_expr(b, c, locals);
            this_expr(d, c, locals);
        }
        ExprKind::Unary(_, a)
        | ExprKind::Member(a, _)
        | ExprKind::NewCopy(a)
        | ExprKind::IncDec { e: a, .. }
        | ExprKind::NamedArg(_, a)
        | ExprKind::TagIs(a, _)
        | ExprKind::Unchecked(a)
        | ExprKind::Tagged(_, Some(a)) => this_expr(a, c, locals),
        ExprKind::Stream { slice, items, .. } => {
            if let Some(x) = slice {
                this_expr(x, c, locals);
            }
            items.iter_mut().for_each(|a| this_expr(a, c, locals));
        }
        ExprKind::AssignExpr { lhs, rhs } => {
            this_expr(lhs, c, locals);
            this_expr(rhs, c, locals);
        }
        ExprKind::Binary(_, a, b) => {
            this_expr(a, c, locals);
            this_expr(b, c, locals);
        }
        ExprKind::Concat(v) | ExprKind::SysCall(_, v) | ExprKind::New(v) | ExprKind::NewTyped(_, v) | ExprKind::StaticCall(_, _, v) => {
            v.iter_mut().for_each(|a| this_expr(a, c, locals))
        }
        ExprKind::Repl(n, v) => {
            this_expr(n, c, locals);
            v.iter_mut().for_each(|a| this_expr(a, c, locals));
        }
        ExprKind::MethodCall(o, _, args) => {
            this_expr(o, c, locals);
            args.iter_mut().for_each(|a| this_expr(a, c, locals));
        }
        ExprKind::ArrayWith { arr, with, .. } => {
            this_expr(arr, c, locals);
            this_expr(with, c, locals);
        }
        ExprKind::Randomize { obj, vars, with, .. } => {
            if let Some(o) = obj {
                this_expr(o, c, locals);
            }
            vars.iter_mut().for_each(|a| this_expr(a, c, locals));
            let mut l = locals.to_vec();
            for it in with.iter_mut().flatten() {
                this_citem(it, c, &mut l);
            }
        }
        ExprKind::NewArray(n, init) => {
            this_expr(n, c, locals);
            if let Some(i) = init {
                this_expr(i, c, locals);
            }
        }
        ExprKind::Pattern(_, items) => {
            for it in items {
                match it {
                    PatItem::Pos(x) | PatItem::Key(_, x) | PatItem::Default(x) | PatItem::Type(_, x) => this_expr(x, c, locals),
                    PatItem::Index(k, x) => {
                        this_expr(k, c, locals);
                        this_expr(x, c, locals);
                    }
                    PatItem::Repl(n, v) => {
                        this_expr(n, c, locals);
                        v.iter_mut().for_each(|x| this_expr(x, c, locals));
                    }
                }
            }
        }
        _ => {}
    }
}

/// A call `p.m(...)` of a method of static property p (`q.push_back(x)`): the name through
/// the property's variable.
fn static_member_call(n: &str, c: &ClassNames, locals: &[String]) -> Option<String> {
    let (p, rest) = n.split_once('.')?;
    Some(format!("{}.{rest}", static_name(p, c, locals)?))
}

/// A call `m(...)` of a static method of the class.
fn static_call(n: &str, c: &ClassNames, locals: &[String]) -> bool {
    c.static_methods.contains(n) && !locals.iter().any(|l| l == n)
}

/// A call `m(...)` / `p.m(...)` of a method through `this`: (object, method).
fn this_call(n: &str, c: &ClassNames, locals: &[String]) -> Option<(Expr, String)> {
    match n.rsplit_once('.') {
        // (a local of the same name, e.g. a function's own result variable, is no
        // subroutine: a call names the method)
        None if c.methods.contains(n) => Some((ExprKind::Ident("this".into()).into(), n.to_string())),
        Some((obj, m)) => {
            let parts: Vec<&str> = obj.split('.').collect();
            if !this_name(seg_name(parts[0]), c, locals) {
                return None;
            }
            let mut x: Option<Expr> = None;
            for p in &parts {
                let base = x.map_or_else(this, Box::new);
                let mut e: Expr = ExprKind::Member(base, seg_name(p).to_string()).into();
                // `q[i].m()`: an element of an array property (`name[@N]` / `name[N]`)
                if let Some(i) = p.strip_suffix(']').and_then(|q| q.split_once('[')).map(|x| x.1) {
                    let mut k = match i.strip_prefix('@') {
                        Some(n) => super::hier_index_expr(n.parse().ok()?)?,
                        None => ExprKind::Num(crate::ast::Number {
                            size: None,
                            signed: true,
                            base: 10,
                            based: false,
                            digits: i.parse::<u64>().ok()?.to_string(),
                            fill: false,
                        })
                        .into(),
                    };
                    this_expr(&mut k, c, locals);
                    e = ExprKind::Index(Box::new(e), Box::new(k)).into();
                }
                x = Some(e);
            }
            Some((x?, m.to_string()))
        }
        None => None,
    }
}

/// The name of a hierarchical segment without its index (`q[@3]` -> `q`).
fn seg_name(p: &str) -> &str {
    p.split_once('[').map_or(p, |x| x.0)
}

/// `this.` before the class's names in a statement; `locals` grows with block declarations.
/// `this_expr` of a constraint item (the variables of `foreach` are locals).
fn this_citem(it: &mut CItem, c: &ClassNames, locals: &mut Vec<String>) {
    match it {
        CItem::Expr { e, .. } | CItem::DisableSoft(e) => this_expr(e, c, locals),
        CItem::Inside { e, set, .. } => {
            this_expr(e, c, locals);
            for (a, b) in set {
                this_expr(a, c, locals);
                if let Some(b) = b {
                    this_expr(b, c, locals);
                }
            }
        }
        CItem::Dist { e, items, .. } => {
            this_expr(e, c, locals);
            for d in items {
                this_expr(&mut d.lo, c, locals);
                if let Some(h) = &mut d.hi {
                    this_expr(h, c, locals);
                }
                this_expr(&mut d.weight, c, locals);
            }
        }
        CItem::Implies(g, v) => {
            this_expr(g, c, locals);
            v.iter_mut().for_each(|x| this_citem(x, c, locals));
        }
        CItem::If(g, a, b) => {
            this_expr(g, c, locals);
            a.iter_mut().chain(b.iter_mut()).for_each(|x| this_citem(x, c, locals));
        }
        CItem::Foreach { arr, vars, items } => {
            this_expr(arr, c, locals);
            let n = locals.len();
            locals.extend(vars.iter().flatten().cloned());
            items.iter_mut().for_each(|x| this_citem(x, c, locals));
            locals.truncate(n);
        }
        CItem::Unique(v) => v.iter_mut().for_each(|x| this_expr(x, c, locals)),
        CItem::SolveBefore => {}
    }
}

fn this_stmt(s: &mut Stmt, c: &ClassNames, locals: &mut Vec<String>) {
    let ex = |e: &mut Expr, l: &[String]| this_expr(e, c, l);
    match s {
        Stmt::Block { decls, stmts, .. } => {
            let n = locals.len();
            for d in decls.iter_mut() {
                for dl in &mut d.names {
                    if let Some(e) = &mut dl.init {
                        this_expr(e, c, locals);
                    }
                    locals.push(dl.name.clone());
                }
            }
            stmts.iter_mut().for_each(|x| this_stmt(x, c, locals));
            locals.truncate(n);
        }
        Stmt::Assign { lhs, rhs, timing, .. } => {
            ex(lhs, locals);
            ex(rhs, locals);
            if let Some(t) = timing {
                this_timing(t, c, locals);
            }
        }
        Stmt::If { cond, then_s, else_s, .. } => {
            ex(cond, locals);
            this_stmt(then_s, c, locals);
            if let Some(e) = else_s {
                this_stmt(e, c, locals);
            }
        }
        Stmt::Case { expr, arms, default, .. } => {
            ex(expr, locals);
            for (labels, body) in arms {
                labels.iter_mut().for_each(|l| ex(l, locals));
                this_stmt(body, c, locals);
            }
            if let Some(d) = default {
                this_stmt(d, c, locals);
            }
        }
        Stmt::For { init, cond, step, body, .. } => {
            this_stmt(init, c, locals);
            ex(cond, locals);
            this_stmt(step, c, locals);
            this_stmt(body, c, locals);
        }
        Stmt::While { cond, body, .. } | Stmt::DoWhile { cond, body, .. } | Stmt::Wait { cond, body, .. } => {
            ex(cond, locals);
            this_stmt(body, c, locals);
        }
        Stmt::Repeat { count, body, .. } => {
            ex(count, locals);
            this_stmt(body, c, locals);
        }
        Stmt::Forever { body, .. } => this_stmt(body, c, locals),
        Stmt::Foreach { array, vars, body, .. } => {
            ex(array, locals);
            let n = locals.len();
            locals.extend(vars.iter().flatten().cloned());
            this_stmt(body, c, locals);
            locals.truncate(n);
        }
        Stmt::Timed { timing, body, .. } => {
            this_timing(timing, c, locals);
            this_stmt(body, c, locals);
        }
        Stmt::SysTask { args, .. } => args.iter_mut().flatten().for_each(|a| ex(a, locals)),
        Stmt::TaskCall { name, args, line } => {
            args.iter_mut().for_each(|a| ex(a, locals));
            if let Some(m) = static_member_call(name, c, locals) {
                *name = m;
            } else if static_call(name, c, locals) {
                *s = Stmt::StaticCall { class: c.name.clone(), name: name.clone(), args: std::mem::take(args), line: *line };
            } else if let Some((obj, m)) = this_call(name, c, locals) {
                *s = Stmt::MethodCall { obj, name: m, args: std::mem::take(args), line: *line };
            }
        }
        Stmt::MethodCall { obj, args, .. } => {
            ex(obj, locals);
            args.iter_mut().for_each(|a| ex(a, locals));
        }
        Stmt::StaticCall { args, .. } => args.iter_mut().for_each(|a| ex(a, locals)),
        Stmt::Force { lhs, rhs, .. } => {
            ex(lhs, locals);
            if let Some(r) = rhs {
                ex(r, locals);
            }
        }
        Stmt::Return { value: Some(v), .. } | Stmt::Discard { e: v, .. } | Stmt::Trigger { target: v, .. } => ex(v, locals),
        _ => {}
    }
}

fn this_timing(t: &mut Timing, c: &ClassNames, locals: &[String]) {
    let ctl = |ec: &mut EventCtl| {
        if let EventCtl::List(v) = ec {
            v.iter_mut().for_each(|(_, e)| this_expr(e, c, locals));
        }
    };
    match t {
        Timing::Delay(e) => this_expr(e, c, locals),
        Timing::Event(ec) => ctl(ec),
        Timing::Repeat(e, ec) => {
            this_expr(e, c, locals);
            ctl(ec);
        }
    }
}
