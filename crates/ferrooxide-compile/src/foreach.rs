//! `foreach` over an array the lowering cannot see (IEEE 1800-2017 12.7.3): a member, an
//! element, a static property, a name declared in another scope. Each loop variable gets a
//! loop over its dimension, chosen from the type of the array: fixed-size dimensions from
//! left to right, queues and dynamic arrays from 0 to `size() - 1`, associative arrays from
//! `first` through `next`.

use crate::types::{DynKind, Type};
use crate::*;

/// How one dimension is iterated.
enum Dim {
    /// a fixed-size dimension: from `left` to `right`
    Fixed { left: i64, right: i64 },
    /// a queue or dynamic array (`size`), a string (`len`)
    Sized(&'static str),
    /// an associative array with this key
    Assoc(rt::KeyKind),
}

fn num(v: i64) -> Expr {
    let e: Expr = ExprKind::Num(dec(v.unsigned_abs())).into();
    if v < 0 {
        ExprKind::Unary(UnOp::Neg, Box::new(e)).into()
    } else {
        e
    }
}

fn ident(n: &str) -> Box<Expr> {
    Box::new(ExprKind::Ident(n.to_string()).into())
}

fn index(a: &Expr, i: Expr) -> Expr {
    ExprKind::Index(Box::new(a.clone()), Box::new(i)).into()
}

/// An automatic variable for a loop.
fn var_decl(kind: NetKind, signed: bool, range: Option<Range>, n: &str, line: Line) -> Decl {
    let names = vec![Declarator { name: n.to_string(), dims: vec![], heap: None, inner: vec![], init: None, line }];
    Decl::var(SvType::flat(kind, signed, range.into_iter().collect()), names, true, line)
}

fn assign(n: &str, rhs: Expr, line: Line) -> Stmt {
    Stmt::Assign { lhs: ExprKind::Ident(n.to_string()).into(), rhs, nonblocking: false, timing: None, line }
}

impl<'a> Compiler<'a> {
    pub(crate) fn foreach(&mut self, array: &Expr, vars: &[Option<String>], body: &Stmt, line: Line, sid: usize) -> R<()> {
        let st = self.foreach_dims(array, array, vars, body, line, sid)?;
        self.stmt(&st, sid)
    }

    /// The loops for `vars` over `sub` (the array with the outer variables as indexes);
    /// `probe` is `sub` with constant indexes instead, to find types before the variables
    /// exist.
    fn foreach_dims(&mut self, sub: &Expr, probe: &Expr, vars: &[Option<String>], body: &Stmt, line: Line, sid: usize) -> R<Stmt> {
        let Some((v, rest)) = vars.split_first() else { return Ok(body.clone()) };
        let dim = self.foreach_dim(probe, sid)?;
        let first = match dim {
            Dim::Fixed { left, .. } => num(left),
            Dim::Sized(_) => num(0),
            Dim::Assoc(_) => num(0),
        };
        let Some(v) = v else {
            // a skipped dimension: the inner ones are the same for every index
            if !matches!(dim, Dim::Fixed { .. }) {
                return self.err("foreach: a skipped dimension of a queue, dynamic or associative array is not supported");
            }
            if rest.iter().all(|r| r.is_none()) {
                return Ok(Stmt::Null);
            }
            return self.foreach_dims(&index(sub, first.clone()), &index(probe, first), rest, body, line, sid);
        };
        let inner = self.foreach_dims(&index(sub, ExprKind::Ident(v.clone()).into()), &index(probe, first), rest, body, line, sid)?;
        let var = || ident(v);
        let int_decl = |n: &str| var_decl(NetKind::Int(32), true, None, n, line);
        let (decls, lp) = match dim {
            Dim::Fixed { left, right } => {
                let (cmp, step) = if left >= right { (BinOp::Ge, BinOp::Sub) } else { (BinOp::Le, BinOp::Add) };
                let lp = Stmt::For {
                    init: Box::new(assign(v, num(left), line)),
                    cond: ExprKind::Binary(cmp, var(), Box::new(num(right))).into(),
                    step: Box::new(assign(v, ExprKind::Binary(step, var(), Box::new(num(1))).into(), line)),
                    body: Box::new(inner),
                    line,
                };
                (vec![int_decl(v)], lp)
            }
            Dim::Sized(m) => {
                // (string methods of a name are calls of `name.len`)
                let size: Expr = match &sub.kind {
                    ExprKind::Ident(n) if m == "len" => ExprKind::Call(format!("{n}.len"), vec![]).into(),
                    _ => ExprKind::MethodCall(Box::new(sub.clone()), m.into(), vec![]).into(),
                };
                let lp = Stmt::For {
                    init: Box::new(assign(v, num(0), line)),
                    cond: ExprKind::Binary(BinOp::Lt, var(), Box::new(size)).into(),
                    step: Box::new(assign(v, ExprKind::Binary(BinOp::Add, var(), Box::new(num(1))).into(), line)),
                    body: Box::new(inner),
                    line,
                };
                (vec![int_decl(v)], lp)
            }
            Dim::Assoc(key) => {
                // the loop variable has the key's type; `ok` the result of first/next
                let ok = format!("{v}$ok");
                let call = |m: &str| -> Expr { ExprKind::MethodCall(Box::new(sub.clone()), m.into(), vec![ExprKind::Ident(v.clone()).into()]).into() };
                let kd = match key {
                    _ if self.struct_key(probe, sid).is_some() => {
                        let names = vec![Declarator { name: v.to_string(), dims: vec![], heap: None, inner: vec![], init: None, line }];
                        Decl::var((*self.struct_key(probe, sid).unwrap()).clone(), names, true, line)
                    }
                    rt::KeyKind::Str => var_decl(NetKind::String, false, None, v, line),
                    rt::KeyKind::Int { w, signed } => var_decl(NetKind::Reg, signed, Some((num(w as i64 - 1), num(0))), v, line),
                };
                let lp = Stmt::For {
                    init: Box::new(assign(&ok, call("first"), line)),
                    cond: ExprKind::Ident(ok.clone()).into(),
                    step: Box::new(assign(&ok, call("next"), line)),
                    body: Box::new(inner),
                    line,
                };
                (vec![kd, int_decl(&ok)], lp)
            }
        };
        Ok(Stmt::Block { name: None, decls, imports: vec![], params: vec![], stmts: vec![lp], fork: None })
    }

    /// The struct or class key type of the associative array property `e` (`obj.p`), if it
    /// has one.
    fn struct_key(&mut self, e: &Expr, sid: usize) -> Option<Rc<SvType>> {
        let ExprKind::Member(b, m) = &e.kind else { return None };
        let mut c = self.handle_class(b, sid).ok()??;
        loop {
            let ci = &self.classes.list[c as usize];
            for d in &ci.decl.props {
                if let Some(n) = d.names.iter().find(|n| n.name == *m) {
                    return match &n.heap {
                        Some(HeapKind::Assoc(Some(k))) if k.has_struct() || k.class.is_some() => Some(k.clone()),
                        _ => None,
                    };
                }
            }
            c = ci.base?;
        }
    }

    /// The outermost unpacked dimension of the array `e`.
    fn foreach_dim(&mut self, e: &Expr, sid: usize) -> R<Dim> {
        // a fixed-size array signal (or its subarray)
        if let Some((_, _, dims)) = self.array_dims(e, sid)? {
            let (lo, n, desc) = dims[0];
            let hi = lo + n as i64 - 1;
            return Ok(if desc { Dim::Fixed { left: hi, right: lo } } else { Dim::Fixed { left: lo, right: hi } });
        }
        // a fixed-size array property of an object (or its subarray)
        let mut base = e;
        let mut nidx = 0;
        while let ExprKind::Index(a, _) = &base.kind {
            base = a;
            nidx += 1;
        }
        if let ExprKind::Member(h, name) = &base.kind {
            if let Type::Class(c) = self.type_of(h, sid)? {
                if c != NONE {
                    let p = self.prop(c, name)?;
                    if nidx < p.dims.len() {
                        let (lo, n) = p.dims[nidx];
                        let hi = lo + n as i64 - 1;
                        return Ok(if p.desc[nidx] { Dim::Fixed { left: hi, right: lo } } else { Dim::Fixed { left: lo, right: hi } });
                    }
                }
            }
        }
        match self.type_of(e, sid)? {
            Type::Dyn { kind: DynKind::Queue(_) | DynKind::Darray, .. } => Ok(Dim::Sized("size")),
            Type::Str => Ok(Dim::Sized("len")),
            Type::Dyn { kind: DynKind::Assoc(k), .. } => Ok(Dim::Assoc(k)),
            _ => self.err("foreach over this expression (no unpacked array) is not supported"),
        }
    }
}
