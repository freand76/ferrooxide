//! Declarations: initial values, implicit nets, views of signals.

use crate::*;

impl<'a> Compiler<'a> {
    /// A new name for signal `base` with its own declared range and sign.
    pub(crate) fn new_view(&mut self, base: u32, name: String, msb: i64, lsb: i64, signed: bool) -> u32 {
        let base = self.canon(base);
        let mut info = self.sig(base).clone();
        info.name = name.clone();
        info.msb = msb;
        info.lsb = lsb;
        info.signed = signed;
        let ty = match self.sig_type(base) {
            types::Type::Vec { w, .. } => types::Type::vec(w, signed),
            t => t,
        };
        let id = self.push_signal(info, ty);
        self.prog.names.push((name, id));
        self.nets.view_of.insert(id, base);
        id
    }

    /// After code generation: instructions, event controls and names refer to the signals the
    /// views stand for.
    pub(crate) fn resolve_views(&mut self) {
        if self.nets.view_of.is_empty() {
            return;
        }
        let canon: Vec<u32> = (0..self.prog.signals.len() as u32).map(|s| self.canon(s)).collect();
        for p in &mut self.prog.procs {
            for ins in &mut p.code {
                rt::instr_sigs_mut(ins, &mut |s| *s = canon[*s as usize]);
            }
        }
        for c in &mut self.prog.ctls {
            for t in &mut c.triggers {
                t.0 = canon[t.0 as usize];
            }
        }
        for n in &mut self.prog.names {
            n.1 = canon[n.1 as usize];
        }
        for (&v, _) in &self.nets.view_of {
            if self.prog.signals[v as usize].forced {
                self.prog.signals[canon[v as usize] as usize].forced = true;
            }
        }
    }

    /// The initializer of a declaration: a continuous assignment for a net, the start value
    /// for a variable.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decl_init(
        &mut self,
        init: &Expr,
        s: u32,
        kind: SigKind,
        nkind: NetKind,
        is_real: bool,
        depth: u32,
        width: u32,
        strength: Option<(u8, u8)>,
        delay: Option<&Expr>,
        name: &str,
        sid: usize,
    ) -> R<()> {
        if kind == SigKind::Wire {
            self.nets.cur_strength = strength;
            self.push_job(Job::Assign {
                lhs: Expr::Ident(name.to_string()),
                rhs: init.clone(),
                delay: delay.cloned(),
                scope: sid,
                line: self.line,
            });
            self.nets.cur_strength = None;
            return Ok(());
        }
        // an initializer that is not a constant expression (6.8: it need not be one) is
        // assigned at time 0, before the processes start
        if self.decl_init_const(init, s, nkind, is_real, depth, width, sid).is_err() {
            let line = self.line;
            self.code.jobs.push(Job::Init { lhs: Expr::Ident(name.to_string()), rhs: init.clone(), scope: sid, line });
        }
        Ok(())
    }

    /// The initial value of variable `s` from a constant initializer, set in memory.
    #[allow(clippy::too_many_arguments)]
    fn decl_init_const(&mut self, init: &Expr, s: u32, nkind: NetKind, is_real: bool, depth: u32, width: u32, sid: usize) -> R<()> {
        if nkind == NetKind::String {
            // the initial text
            let text = match init {
                Expr::Str(t) => t.chars().map(|c| c as u32 as u8).filter(|&c| c != 0).collect(),
                e => {
                    let c = self.const_eval(e, sid)?;
                    let mut v = Vec::new();
                    for k in (0..c.w.div_ceil(8)).rev() {
                        let ch = (c.a[(8 * k / 64) as usize] >> (8 * k % 64)) as u8;
                        if ch != 0 {
                            v.push(ch);
                        }
                    }
                    v
                }
            };
            let h = self.intern(&text);
            let slot = self.sig(s).slot as usize;
            self.prog.mem[slot] = h;
            self.prog.mem[slot + 1] = 0;
            self.code.init_touch.push(s);
        } else if is_real {
            let r = self.const_real(init, sid)?;
            let slot = self.sig(s).slot as usize;
            self.prog.mem[slot] = r.to_bits();
            self.prog.mem[slot + 1] = 0;
            self.code.init_touch.push(s);
        } else {
            if depth == 0 {
                self.code.init_touch.push(s);
            }
            // an assignment pattern for the variable's type
            let layout = if depth == 0 { self.layout(s) } else { None };
            let r = self.with_target(init, layout.as_deref(), sid)?;
            let init = r.as_ref().unwrap_or(init);
            let c = self.const_eval(init, sid)?;
            let c = c.resize(width, c.signed);
            let slot = self.sig(s).slot as usize;
            let n = rt::nw(width);
            for i in 0..n {
                self.prog.mem[slot + i] = c.a[i];
                self.prog.mem[slot + n + i] = c.b[i];
            }
        }
        Ok(())
    }

    /// An implicitly declared net (`default_nettype; `none` makes it an error).
    pub(crate) fn implicit_net(&mut self, n: &str, width: u32, sid: usize) -> R<u32> {
        let nt = self.scopes[sid].nettype.clone();
        let net = match nt.as_deref() {
            None => NetType::Wire,
            Some("none") => return self.err(format!("undeclared identifier '{n}' (`default_nettype none)")),
            Some("wand" | "triand") => NetType::Wand,
            Some("wor" | "trior") => NetType::Wor,
            Some("tri0") => NetType::Tri0,
            Some("tri1") => NetType::Tri1,
            Some("supply0") => NetType::Supply0,
            Some("supply1") => NetType::Supply1,
            Some(_) => NetType::Wire,
        };
        let full = format!("{}.{}", self.scopes[sid].name, n);
        let s = self.new_signal(full, width, false, 0, width as i64 - 1, 0, 0, SigKind::Wire);
        self.scopes[sid].syms.insert(n.to_string(), Sym::Sig(s));
        if net != NetType::Wire {
            self.nets.net_types.insert(s, net);
        }
        Ok(s)
    }
}
