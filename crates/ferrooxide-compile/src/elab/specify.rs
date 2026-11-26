//! Specify blocks: path delays.

use crate::*;

impl<'a> Compiler<'a> {
    /// The module paths are valid (whether or not their delays apply): a parallel path
    /// (`=>`) connects a source and a destination of the same width (IEEE 1800-2017 30.4.5).
    pub(crate) fn check_paths(&mut self, items: &'a [Item], sid: usize) -> R<()> {
        for it in items {
            let Item::SpecPath { srcs, dsts, parallel: true, line, .. } = it else { continue };
            self.line = *line;
            if let ([s], [d]) = (&srcs[..], &dsts[..]) {
                if let (Some(Sym::Sig(a)), Some(Sym::Sig(b))) = (self.lookup(s, sid), self.lookup(d, sid)) {
                    let (wa, wb) = (self.sig(a).width, self.sig(b).width);
                    if wa != wb {
                        return self.err(format!("a parallel path (`=>`) between '{s}' ({wa} bits) and '{d}' ({wb} bits) of different widths"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Module path delays (`specify`): an output port with paths is driven, through an inertial
    /// delay, from an internal copy that the module's logic drives; the delay is that of the path
    /// whose source changed last (each source records the time of its last change).
    pub(crate) fn path_delays(&mut self, items: &'a [Item], sid: usize, dirs: &HashMap<String, Dir>) -> R<()> {
        // destination -> (source, delay) pairs
        let mut by_dst: Vec<(String, Vec<(String, Expr)>)> = Vec::new();
        for it in items {
            let Item::SpecPath { srcs, dsts, delay, line, .. } = it else { continue };
            self.line = *line;
            for d in dsts {
                if dirs.get(d) != Some(&Dir::Output) {
                    continue; // only output ports (inout paths are not modelled)
                }
                let e = match by_dst.iter_mut().find(|x| &x.0 == d) {
                    Some(e) => e,
                    None => {
                        by_dst.push((d.clone(), Vec::new()));
                        by_dst.last_mut().unwrap()
                    }
                };
                for s in srcs {
                    e.1.push((s.clone(), delay.clone()));
                }
            }
        }
        if by_dst.is_empty() {
            return Ok(());
        }
        let mut times: HashMap<String, String> = HashMap::new();
        for (dst, paths) in by_dst {
            let Some(Sym::Sig(port)) = self.scopes[sid].syms.get(&dst).cloned() else { continue };
            // the time of each source's last change
            for (src, _) in &paths {
                if times.contains_key(src) || self.lookup(src, sid).is_none() {
                    continue;
                }
                let tname = format!("$fxt_{src}");
                let full = format!("{}.{tname}", self.scopes[sid].name);
                let t = self.new_signal(full, 64, false, 0, 63, 0, 0, SigKind::Reg);
                self.prog.names.pop();
                self.zero_init(t);
                self.scopes[sid].syms.insert(tname.clone(), Sym::Sig(t));
                let body: &'a Stmt = Box::leak(Box::new(Stmt::Timed {
                    timing: Timing::Event(EventCtl::List(vec![(Edge::Any, ExprKind::Ident(src.clone()).into())])),
                    body: Box::new(Stmt::Assign {
                        lhs: ExprKind::Ident(tname.clone()).into(),
                        rhs: ExprKind::SysCall("$time".into(), vec![]).into(),
                        nonblocking: false,
                        timing: None,
                        line: self.line,
                    }),
                    line: self.line,
                }));
                self.code.jobs.push(Job::Proc { kind: ProcKind::Always, body, scope: sid, line: self.line });
                times.insert(src.clone(), tname);
            }
            let paths: Vec<(String, Expr)> = paths.into_iter().filter(|(s, _)| times.contains_key(s)).collect();
            if paths.is_empty() {
                continue;
            }
            // the internal copy the module's logic drives
            let info = self.sig(port).clone();
            let full = format!("{}.{dst}$int", self.scopes[sid].name);
            // (a variable for a variable port: the module may assign it procedurally)
            let kind = if info.kind == SigKind::Wire { SigKind::Wire } else { SigKind::Reg };
            let int = self.new_signal(full, info.width, info.signed, 0, info.msb, info.lsb, 0, kind);
            self.prog.names.pop();
            let pname = format!("$fxport_{dst}");
            self.scopes[sid].syms.insert(dst.clone(), Sym::Sig(int));
            self.scopes[sid].syms.insert(pname.clone(), Sym::Sig(port));
            // (rise, fall, turn-off) of each path
            let comps = |d: &Expr| -> [Expr; 3] {
                match &d.kind {
                    ExprKind::SysCall(n, v) if n == "$__delays" => {
                        let t = v.get(2).cloned().unwrap_or_else(|| {
                            ExprKind::Cond(
                                Box::new(ExprKind::Binary(BinOp::Lt, Box::new(v[0].clone()), Box::new(v[1].clone())).into()),
                                Box::new(v[0].clone()),
                                Box::new(v[1].clone()),
                            ).into()
                        });
                        [v[0].clone(), v[1].clone(), t]
                    }
                    d => [d.clone().into(), d.clone().into(), d.clone().into()],
                }
            };
            let tvar = |s: &str| Expr::from(ExprKind::Ident(times[s].clone()));
            let mut sel: [Expr; 3] = comps(&paths[0].1);
            let mut tmax = tvar(&paths[0].0);
            for (src, d) in &paths[1..] {
                let newer = Box::new(Expr::from(ExprKind::Binary(BinOp::Gt, Box::new(tvar(src)), Box::new(tmax.clone()))));
                let c = comps(d);
                for k in 0..3 {
                    sel[k] = ExprKind::Cond(newer.clone(), Box::new(c[k].clone()), Box::new(sel[k].clone())).into();
                }
                tmax = ExprKind::Cond(newer, Box::new(tvar(src)), Box::new(tmax)).into();
            }
            // a multi-bit output: the LSB's transition picks the delay for all bits (as Icarus)
            let multi = info.width > 1;
            if multi {
                let w = "sorry: multi-bit module path delays are currently not fully supported.\n     : using the LSB delay for all bits.".to_string();
                if self.diag.warned.insert(w.clone()) {
                    self.diag.warnings.push(w);
                }
            }
            let delay = ExprKind::SysCall(if multi { "$__pathdelays" } else { "$__delays" }.into(), sel.to_vec());
            self.push_job(Job::Assign {
                lhs: ExprKind::Ident(pname).into(),
                rhs: ExprKind::Ident(dst.clone()).into(),
                delay: Some(delay.into()),
                scope: sid,
                line: self.line,
            });
        }
        Ok(())
    }
}
