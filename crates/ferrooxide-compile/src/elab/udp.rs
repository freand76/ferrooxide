//! User-defined primitives: tables and instances.

use crate::*;

impl<'a> Compiler<'a> {
    /// The runtime table of a UDP (converted once per primitive).
    pub(crate) fn udp_table(&mut self, m: &Module) -> R<u32> {
        if let Some(&i) = self.code.udp_index.get(&m.name) {
            return Ok(i);
        }
        let def = m.udp.as_ref().unwrap();
        let mask = |c: &str| -> Option<u8> {
            Some(match c {
                "0" => rt::UDP_0,
                "1" => rt::UDP_1,
                "x" => rt::UDP_X,
                "?" => rt::UDP_0 | rt::UDP_1 | rt::UDP_X,
                "b" => rt::UDP_0 | rt::UDP_1,
                _ => return None,
            })
        };
        let mut rows = Vec::new();
        for (ins, state, out) in &def.rows {
            let mut syms = Vec::new();
            let mut has_edge = false;
            for s in ins {
                let any = rt::UDP_0 | rt::UDP_1 | rt::UDP_X;
                let sym = match s.as_str() {
                    "r" => rt::UdpSym { edge: true, prev: rt::UDP_0, cur: rt::UDP_1 },
                    "f" => rt::UdpSym { edge: true, prev: rt::UDP_1, cur: rt::UDP_0 },
                    "p" => rt::UdpSym { edge: true, prev: rt::UDP_0 | rt::UDP_X, cur: rt::UDP_1 | rt::UDP_X },
                    "n" => rt::UdpSym { edge: true, prev: rt::UDP_1 | rt::UDP_X, cur: rt::UDP_0 | rt::UDP_X },
                    "*" => rt::UdpSym { edge: true, prev: any, cur: any },
                    e if e.starts_with('(') && e.len() == 4 => {
                        let (a, b) = (mask(&e[1..2]), mask(&e[2..3]));
                        match (a, b) {
                            (Some(a), Some(b)) => rt::UdpSym { edge: true, prev: a, cur: b },
                            _ => return self.err(format!("bad edge '{e}' in table of '{}'", m.name)),
                        }
                    }
                    l => match mask(l) {
                        Some(c) => rt::UdpSym { edge: false, prev: 0, cur: c },
                        None => return self.err(format!("bad symbol '{l}' in table of '{}'", m.name)),
                    },
                };
                has_edge |= sym.edge;
                syms.push(sym);
            }
            if syms.len() != m.ports.len() - 1 {
                return self.err(format!("table row of '{}' has {} inputs, expected {}", m.name, syms.len(), m.ports.len() - 1));
            }
            let st = match state {
                Some(s) => mask(s).ok_or_else(|| self.mkerr(format!("bad state '{s}' in table of '{}'", m.name)))?,
                None => 7,
            };
            let out = match out.as_str() {
                "0" => 0,
                "1" => 1,
                "x" => 2,
                "-" => 3,
                o => return self.err(format!("bad output '{o}' in table of '{}'", m.name)),
            };
            rows.push(rt::UdpRow { ins: syms, state: st, out, has_edge });
        }
        let i = self.prog.udps.len() as u32;
        self.prog.udps.push(rt::UdpTable { sequential: def.sequential, rows });
        self.code.udp_index.insert(m.name.clone(), i);
        Ok(i)
    }

    /// A process for a UDP instance: evaluate the table on every input change, drive the output.
    pub(crate) fn udp_instance(&mut self, m: &'a Module, terms: &[Expr], delay: Option<&Expr>, sid: usize) -> R<()> {
        let table = self.udp_table(m)?;
        let def = m.udp.as_ref().unwrap();
        let name = self.scopes[sid].name.clone();
        self.begin_proc(name, ProcClass::Normal);
        // state / output and previous inputs persist between activations (start as x)
        let set = |c: &mut Self, s: u32, v: (u64, u64)| {
            c.prog.mem[s as usize] = v.0;
            c.prog.mem[s as usize + 1] = v.1;
        };
        let q = self.ptmp(1);
        let init = match &def.init {
            Some(e) => {
                let c = self.const_eval(e, sid)?;
                (c.a[0] & 1, c.b[0] & 1)
            }
            None => (1, 1),
        };
        set(self, q, init);
        let mut prev = Vec::new();
        for _ in 1..terms.len() {
            let s = self.ptmp(1);
            set(self, s, (1, 1));
            prev.push(s);
        }
        // a UDP output starts as x (not z) until the table drives it
        let mut outs = Vec::new();
        self.lvalue_bases(&terms[0], sid, &mut outs);
        for o in outs {
            let i = self.sig(o).clone();
            if i.kind == SigKind::Wire && i.depth == 0 {
                let n = rt::nw(i.width);
                for k in 0..n {
                    let m = if k == n - 1 { rt::tmask(i.width) } else { !0 };
                    self.prog.mem[i.slot as usize + k] = m;
                    self.prog.mem[i.slot as usize + n + k] = m;
                }
            }
        }
        let mut reads = Vec::new();
        for e in &terms[1..] {
            self.reads_expr(e, sid, &mut reads);
        }
        let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, rt::Edge::Any, e)).collect());
        if def.init.is_some() {
            // an `initial` output value: a change at time 0 after the other processes have
            // started (as in the reference); the table is evaluated from the first input change
            let p = self.code.pb.last().unwrap().proc;
            self.prog.procs[p as usize].kick0 = true;
            self.emit(Instr::Wait { ctl });
            self.store_value(&stmt::Lhs::Expr(&terms[0], sid), q, 1, rt::StoreMode::Blocking)?;
            self.emit(Instr::Wait { ctl });
        }
        let top = self.here();
        let mut ins = Vec::new();
        for e in &terms[1..] {
            let v = self.rhs_value(e, sid, 1)?;
            let t = self.tmp(1);
            self.emit(Instr::Copy { d: t, s: v, w: 1 });
            ins.push(t);
        }
        self.emit(Instr::Udp { table, ins: ins.into_boxed_slice(), prev: prev.into_boxed_slice(), q });
        if let Some(d) = delay {
            // the output changes `d` later (inertial, as a gate's); evaluation goes on meanwhile
            let ds = self.delays3(d, sid)?;
            self.store_value(&stmt::Lhs::Expr(&terms[0], sid), q, 1, rt::StoreMode::Inertial(ds))?;
        } else {
            self.store_value(&stmt::Lhs::Expr(&terms[0], sid), q, 1, rt::StoreMode::Blocking)?;
        }
        self.emit(Instr::Wait { ctl });
        self.emit(Instr::Jmp { to: top });
        self.end_proc();
        Ok(())
    }
}
