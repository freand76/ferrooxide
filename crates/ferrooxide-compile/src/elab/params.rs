//! Parameters: values, overrides, defparams, type parameters.

use crate::*;

impl<'a> Compiler<'a> {
    /// Evaluate the parameters of module `m` (with overrides) into scope `sid`, in order.
    pub(crate) fn eval_params(&mut self, m: &'a Module, overrides: &[(Option<String>, PVal)], sid: usize) -> R<()> {
        let mut pos = 0usize;
        for it in &m.items {
            if let Item::TypeParam { local, name, default, line } = it {
                self.line = *line;
                let mut ov = None;
                if !local {
                    ov = overrides.iter().find(|(n, _)| n.as_deref() == Some(name.as_str())).map(|x| x.1.clone());
                    if ov.is_none() {
                        let positional: Vec<&PVal> = overrides.iter().filter(|(n, _)| n.is_none()).map(|(_, c)| c).collect();
                        if pos < positional.len() {
                            ov = Some(positional[pos].clone());
                        }
                    }
                    pos += 1;
                }
                let t = match (ov, default) {
                    (Some(PVal::Type(t)), _) => t,
                    (Some(_), _) => return self.err(format!("type parameter '{name}' needs a type")),
                    (None, Some(d)) => Rc::new(self.resolve_type(d, sid)?),
                    (None, None) => return self.err(format!("type parameter '{name}' has no type")),
                };
                self.scopes[sid].syms.insert(name.clone(), Sym::Type(t));
                continue;
            }
            if let Item::Param(p) = it {
                self.line = p.line;
                let mut ov = None;
                if !p.local {
                    ov = overrides.iter().find(|(n, _)| n.as_deref() == Some(p.name.as_str())).map(|x| x.1.clone());
                    if ov.is_none() {
                        let positional: Vec<&PVal> =
                            overrides.iter().filter(|(n, _)| n.is_none()).map(|(_, c)| c).collect();
                        if pos < positional.len() {
                            ov = Some(positional[pos].clone());
                        }
                    }
                    pos += 1;
                }
                if !p.local {
                    let key = format!("{}.{}", self.scopes[sid].name, p.name);
                    if let Some(d) = self.hier.defparams.get(&key) {
                        ov = Some(d.clone());
                    }
                    self.hier.params_done.insert(key);
                }
                let v = self.param_value(p, ov, sid)?;
                self.insert_param(p, v, sid)?;
                if p.range.is_some() && !p.real {
                    let r = self.range_vals(&p.range, sid)?;
                    self.scopes[sid].param_ranges.insert(p.name.clone(), r);
                }
            }
        }
        Ok(())
    }

    /// Record the defparams among `items` (in scope `sid`) by the full path of their target.
    pub(crate) fn register_defparams(&mut self, items: &'a [Item], sid: usize) -> R<()> {
        for it in items {
            let Item::Defparam { target, value, line } = it else { continue };
            self.line = *line;
            // generate-loop indices in the path (`Loop[i].m.p`)
            let target = &match self.hier_parts(target, sid) {
                Some(t) => t,
                None => return self.err(format!("defparam: cannot evaluate '{}'", target.join("."))),
            };
            let v = self.pval(value, sid)?;
            let rel = target.join(".");
            // upwards: the first component names this scope or an enclosing one
            let mut base = None;
            let mut s = Some(sid);
            while let Some(x) = s {
                let name = &self.scopes[x].name;
                if name.rsplit('.').next() == Some(target[0].as_str()) {
                    base = Some(match name.rfind('.') {
                        Some(i) => format!("{}.{rel}", &name[..i]),
                        None => rel.clone(),
                    });
                    break;
                }
                s = self.scopes[x].parent;
            }
            // relative: to this scope if it declares the first name (an instance or generate
            // block), else to the nearest enclosing scope that has it, else to the module
            let first = target[0].split('[').next().unwrap_or("");
            let declares = |its: &[Item]| {
                its.iter().any(|i| match i {
                    Item::Instances { insts, .. } => insts.iter().any(|x| x.name == first),
                    Item::GenFor { name: Some(name), .. } | Item::GenBlock { name: Some(name), .. } => name == first,
                    Item::GenIf { .. } | Item::GenCase { .. } => true,
                    _ => false,
                })
            };
            let rel_base = if declares(items) {
                sid
            } else {
                let mut x = sid;
                loop {
                    if self.scopes[x].children.contains_key(&target[0]) {
                        break x;
                    }
                    match self.scopes[x].parent {
                        Some(p) => x = p,
                        None => break x,
                    }
                }
            };
            let key = match base {
                Some(k) => k,
                None if self.hier.top_names.contains(&target[0]) && target.len() > 1 => rel,
                None => format!("{}.{rel}", self.scopes[rel_base].name),
            };
            if !self.hier.restarted && self.hier.params_done.contains(&key) && !self.hier.defparams.contains_key(&key) {
                self.hier.late_defparam = true;
            }
            self.hier.defparams.insert(key, v);
        }
        Ok(())
    }

    /// The value of a parameter (from its override if any): integer or real.
    pub(crate) fn param_value(&mut self, p: &ParamDecl, ov: Option<PVal>, sid: usize) -> R<Sym> {
        let untyped = !p.integer && p.range.is_none() && !p.signed;
        // a number given for a ranged parameter: widened like in an expression (`'bx` fills)
        if let (None, Expr::Num(n), Some(_), false) = (&ov, &p.value, &p.range, p.real) {
            let (a, b) = self.range_vals(&p.range, sid)?;
            let w = ((a - b).abs() + 1) as u32;
            let c = Const::number_in(n, w, n.signed).or_else(|m| self.err(m))?;
            return Ok(Sym::Const(c.with_sign(p.signed)));
        }
        let v = match ov {
            Some(v) => v,
            None => {
                let layout = match &p.ty {
                    Some(t) => Some(self.packed(t, sid)?),
                    None => None,
                };
                let r = self.with_target(&p.value, layout.as_ref(), sid)?;
                let value = r.as_ref().unwrap_or(&p.value);
                if self.type_of(value, sid)?.is_real() {
                    PVal::Real(self.const_real(value, sid)?)
                } else if self.is_str_valued(value, sid) {
                    PVal::Str(self.const_eval(value, sid)?)
                } else {
                    PVal::Int(self.const_eval(value, sid)?)
                }
            }
        };
        if matches!(v, PVal::Type(_)) {
            return self.err(format!("parameter '{}' needs a value, not a type", p.name));
        }
        if p.real || (untyped && matches!(v, PVal::Real(_))) {
            let r = match v {
                PVal::Real(r) => r,
                PVal::Int(c) | PVal::Str(c) => c.to_i64().unwrap_or(0) as f64,
                PVal::Type(_) => unreachable!(),
            };
            return Ok(Sym::Real(r));
        }
        if matches!(v, PVal::Str(_)) {
            self.scopes[sid].str_params.insert(p.name.clone());
        }
        let c = match v {
            PVal::Int(c) | PVal::Str(c) => c,
            PVal::Real(r) => Const::from_i64(r.round() as i64, 32, true),
            PVal::Type(_) => unreachable!(),
        };
        Ok(Sym::Const(self.param_type(p, c, sid)?))
    }

    /// Declare parameter `p` with value `v` in scope `sid`; one with a struct type also gets
    /// its type (`$type:p`, for member selects) and its flat name.
    pub(crate) fn insert_param(&mut self, p: &ParamDecl, v: Sym, sid: usize) -> R<()> {
        if let Some(t) = p.ty.as_ref().filter(|t| t.has_struct()) {
            let t = self.resolve_type(t, sid)?;
            self.scopes[sid].syms.insert(format!("$type:{}", p.name), Sym::Type(Rc::new(t)));
            self.scopes[sid].syms.insert(format!("$flat:{}", p.name), v.clone());
            self.sigs.has_structs = true;
        }
        self.scopes[sid].syms.insert(p.name.clone(), v);
        Ok(())
    }

    /// evaluate a parameter override in the instantiating scope
    pub(crate) fn pval(&mut self, e: &Expr, sid: usize) -> R<PVal> {
        if let Expr::Type(t) = e {
            return Ok(PVal::Type(Rc::new(self.resolve_type(t, sid)?)));
        }
        if self.type_of(e, sid)?.is_real() {
            Ok(PVal::Real(self.const_real(e, sid)?))
        } else if self.is_str_valued(e, sid) {
            Ok(PVal::Str(self.const_eval(e, sid)?))
        } else {
            Ok(PVal::Int(self.const_eval(e, sid)?))
        }
    }

    /// `e` is a string literal, a concatenation of such, or a parameter with one.
    pub(crate) fn is_str_valued(&self, e: &Expr, sid: usize) -> bool {
        match e {
            Expr::Str(_) => true,
            Expr::Concat(v) => !v.is_empty() && v.iter().all(|x| self.is_str_valued(x, sid)),
            Expr::Ident(n) => self.is_str_param(n, sid),
            _ => false,
        }
    }

    /// `n` names a parameter whose value is a string literal (see `Scope::str_params`).
    pub(crate) fn is_str_param(&self, n: &str, mut sid: usize) -> bool {
        if let Some((p, n)) = n.split_once("::") {
            return self.hier.pkg_scopes.get(p).is_some_and(|&s| self.scopes[s].str_params.contains(n));
        }
        loop {
            if self.scopes[sid].syms.contains_key(n) {
                return self.scopes[sid].str_params.contains(n);
            }
            for &p in &self.scopes[sid].imports {
                if self.scopes[p].syms.contains_key(n) {
                    return self.scopes[p].str_params.contains(n);
                }
            }
            match self.scopes[sid].parent {
                Some(p) => sid = p,
                None => return false,
            }
        }
    }

    /// A data type with its dimensions evaluated in scope `sid` (type parameters replaced by
    /// their types there).
    pub(crate) fn resolve_type(&mut self, t: &SvType, sid: usize) -> R<SvType> {
        let lit = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
            if v < 0 { Expr::Unary(UnOp::Neg, Box::new(n)) } else { n }
        };
        let mut dims = Vec::new();
        for (a, b) in &t.dims {
            dims.push((lit(self.const_int(a, sid)?), lit(self.const_int(b, sid)?)));
        }
        let mut unpacked = Vec::new();
        for (a, b) in &t.unpacked {
            unpacked.push((lit(self.const_int(a, sid)?), lit(self.const_int(b, sid)?)));
        }
        if let Some(p) = &t.param {
            let Some(Sym::Type(base)) = self.lookup(p, sid) else {
                return self.err(format!("'{p}' is not a type"));
            };
            let mut r = (*base).clone();
            if !dims.is_empty() {
                r.int_kind = false;
                dims.extend(r.dims);
                r.dims = dims;
            }
            unpacked.extend(r.unpacked);
            r.unpacked = unpacked;
            return Ok(r);
        }
        let mut r = t.clone();
        r.dims = dims;
        r.unpacked = unpacked;
        if let Some(f) = &t.fields {
            let mut f2 = (**f).clone();
            for (_, ft) in &mut f2.fields {
                *ft = self.resolve_type(ft, sid)?;
            }
            r.fields = Some(Rc::new(f2));
        }
        Ok(r)
    }

    pub(crate) fn param_type(&mut self, p: &ParamDecl, v: Const, sid: usize) -> R<Const> {
        if p.integer {
            return Ok(v.resize(32, v.signed).with_sign(true));
        }
        if p.range.is_some() {
            let (a, b) = self.range_vals(&p.range, sid)?;
            let w = ((a - b).abs() + 1) as u32;
            return Ok(v.resize(w, v.signed).with_sign(p.signed));
        }
        if p.signed {
            return Ok(v.with_sign(true));
        }
        Ok(v)
    }
}
