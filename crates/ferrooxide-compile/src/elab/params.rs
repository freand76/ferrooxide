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
                    if let Some(d) = self.defparams.get(&key) {
                        ov = Some(d.clone());
                    }
                    self.params_done.insert(key);
                }
                let v = self.param_value(p, ov, sid)?;
                self.scopes[sid].syms.insert(p.name.clone(), v);
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
                None if self.top_names.contains(&target[0]) && target.len() > 1 => rel,
                None => format!("{}.{rel}", self.scopes[rel_base].name),
            };
            if !self.restarted && self.params_done.contains(&key) && !self.defparams.contains_key(&key) {
                self.late_defparam = true;
            }
            self.defparams.insert(key, v);
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
                if self.is_real(&p.value, sid) {
                    PVal::Real(self.const_real(&p.value, sid)?)
                } else {
                    PVal::Int(self.const_eval(&p.value, sid)?)
                }
            }
        };
        if matches!(v, PVal::Type(_)) {
            return self.err(format!("parameter '{}' needs a value, not a type", p.name));
        }
        if p.real || (untyped && matches!(v, PVal::Real(_))) {
            let r = match v {
                PVal::Real(r) => r,
                PVal::Int(c) => c.to_i64().unwrap_or(0) as f64,
                PVal::Type(_) => unreachable!(),
            };
            return Ok(Sym::Real(r));
        }
        let c = match v {
            PVal::Int(c) => c,
            PVal::Real(r) => Const::from_i64(r.round() as i64, 32, true),
            PVal::Type(_) => unreachable!(),
        };
        Ok(Sym::Const(self.param_type(p, c, sid)?))
    }

    /// evaluate a parameter override in the instantiating scope
    pub(crate) fn pval(&mut self, e: &Expr, sid: usize) -> R<PVal> {
        if let Expr::Type(t) = e {
            return Ok(PVal::Type(Rc::new(self.resolve_type(t, sid)?)));
        }
        if self.is_real(e, sid) {
            Ok(PVal::Real(self.const_real(e, sid)?))
        } else {
            Ok(PVal::Int(self.const_eval(e, sid)?))
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
