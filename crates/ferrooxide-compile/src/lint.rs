//! Lint checks (`--lint`).
//!
//! Incomplete sensitivity lists: a level-sensitive `always @(a or b ...)` block that reads a
//! signal not in its list only re-evaluates when a listed signal changes, while synthesis builds
//! combinational logic that follows every input. Simulation and hardware then disagree.
//!
//! Names that are SystemVerilog keywords (legal Verilog-2005, rejected by SystemVerilog tools)
//! are recorded by the parser and reported in `compile_dialect`.

use crate::*;

impl<'a> Compiler<'a> {
    /// Check an `always` block with an explicit, purely level-sensitive event list.
    pub(crate) fn lint_sensitivity(&mut self, items: &[(Edge, Expr)], body: &Stmt, sid: usize, line: u32) {
        if items.is_empty() || items.iter().any(|(e, _)| *e != Edge::Any) {
            return;
        }
        let mut listed: HashSet<u32> = HashSet::new();
        for (_, e) in items {
            let mut v = Vec::new();
            match e {
                Expr::Ident(_) | Expr::Hier(_) => self.lvalue_bases(e, sid, &mut v),
                _ => {
                    let mut r = Vec::new();
                    self.reads_expr(e, sid, &mut r);
                    v.extend(r.into_iter().map(|x| x.0));
                }
            }
            listed.extend(v);
        }
        let mut reads = Vec::new();
        self.func_reads.set(true);
        self.reads_stmt(body, sid, &mut reads);
        self.func_reads.set(false);
        // variables the block computes itself (temporaries) are not inputs
        let mut written = Vec::new();
        self.blocking_writes(body, sid, &mut written);
        let mut missing: Vec<String> = reads
            .into_iter()
            .map(|(s, _)| s)
            .filter(|s| !listed.contains(s) && !written.contains(s))
            .map(|s| self.short_name(s, sid))
            .collect();
        missing.sort();
        missing.dedup();
        if missing.is_empty() {
            return;
        }
        let list = missing.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(", ");
        let msg = format!(
            "{}:{}: lint: incomplete sensitivity list: {} {} read but not in the list \
             (synthesis treats the block as combinational logic of all its inputs; use @*)",
            self.file,
            line,
            list,
            if missing.len() == 1 { "is" } else { "are" }
        );
        if self.lint_seen.insert(msg.clone()) {
            self.lint.push(msg);
        }
    }

    /// The name of a signal as seen from scope `sid` (module-local names without the path).
    fn short_name(&self, s: u32, sid: usize) -> String {
        // the name the block uses for it (ports collapsed onto a parent signal keep their own name)
        let mut sc = Some(sid);
        while let Some(x) = sc {
            let mut names: Vec<&String> = self.scopes[x]
                .syms
                .iter()
                .filter(|(_, v)| matches!(v, Sym::Sig(t) if *t == s))
                .map(|(k, _)| k)
                .collect();
            names.sort();
            if let Some(n) = names.first() {
                return n.to_string();
            }
            sc = self.scopes[x].parent;
        }
        let full = &self.sig(s).name;
        let mut sc = Some(sid);
        while let Some(x) = sc {
            let prefix = format!("{}.", self.scopes[x].name);
            if let Some(rest) = full.strip_prefix(&prefix) {
                return rest.to_string();
            }
            sc = self.scopes[x].parent;
        }
        full.clone()
    }

    /// Signals written with blocking assignments anywhere in a statement.
    pub(crate) fn blocking_writes(&self, st: &Stmt, sid: usize, out: &mut Vec<u32>) {
        match st {
            Stmt::Assign { lhs, nonblocking: false, .. } => self.lvalue_bases(lhs, sid, out),
            Stmt::Block { stmts, .. } => {
                for s in stmts {
                    self.blocking_writes(s, sid, out);
                }
            }
            Stmt::If { then_s, else_s, .. } => {
                self.blocking_writes(then_s, sid, out);
                if let Some(e) = else_s {
                    self.blocking_writes(e, sid, out);
                }
            }
            Stmt::Case { arms, default, .. } => {
                for (_, b) in arms {
                    self.blocking_writes(b, sid, out);
                }
                if let Some(d) = default {
                    self.blocking_writes(d, sid, out);
                }
            }
            Stmt::For { init, step, body, .. } => {
                self.blocking_writes(init, sid, out);
                self.blocking_writes(step, sid, out);
                self.blocking_writes(body, sid, out);
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Forever { body, .. }
            | Stmt::Timed { body, .. }
            | Stmt::Wait { body, .. } => self.blocking_writes(body, sid, out),
            _ => {}
        }
    }
}
