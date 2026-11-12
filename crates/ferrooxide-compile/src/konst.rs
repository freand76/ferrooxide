//! Compile-time 4-state constants.

use ferrooxide_front::ast::Number;
use ferrooxide_rt::{nw, tmask};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Const {
    pub w: u32,
    pub signed: bool,
    pub a: Vec<u64>,
    pub b: Vec<u64>,
}

impl Const {
    pub fn zero(w: u32) -> Const {
        Const { w, signed: false, a: vec![0; nw(w)], b: vec![0; nw(w)] }
    }

    pub fn from_i64(v: i64, w: u32, signed: bool) -> Const {
        let n = nw(w);
        let mut a = vec![if v < 0 { !0 } else { 0 }; n];
        a[0] = v as u64;
        let mut c = Const { w, signed, a, b: vec![0; n] };
        c.mask();
        c
    }

    pub fn from_str_lit(s: &str) -> Const {
        let bytes = s.as_bytes();
        let w = (8 * bytes.len()).max(8) as u32;
        let mut c = Const::zero(w);
        for (i, &byte) in bytes.iter().rev().enumerate() {
            let bit = i * 8;
            c.a[bit / 64] |= (byte as u64) << (bit % 64);
        }
        c
    }

    fn mask(&mut self) {
        let n = nw(self.w);
        self.a.resize(n, 0);
        self.b.resize(n, 0);
        self.a[n - 1] &= tmask(self.w);
        self.b[n - 1] &= tmask(self.w);
    }

    pub fn is_known(&self) -> bool {
        self.b.iter().all(|&x| x == 0)
    }

    pub fn bit(&self, i: u32) -> (u64, u64) {
        ((self.a[(i / 64) as usize] >> (i % 64)) & 1, (self.b[(i / 64) as usize] >> (i % 64)) & 1)
    }

    pub fn to_i64(&self) -> Option<i64> {
        if !self.is_known() {
            return None;
        }
        let v = self.a[0];
        if self.signed && self.w < 64 {
            let sh = 64 - self.w;
            return Some(((v << sh) as i64) >> sh);
        }
        Some(v as i64)
    }

    pub fn is_true(&self) -> bool {
        self.a.iter().zip(&self.b).any(|(a, b)| a & !b != 0)
    }

    pub fn resize(&self, w: u32, sign: bool) -> Const {
        let n = nw(w);
        let msb = if self.w > 0 { self.bit(self.w - 1) } else { (0, 0) };
        let mut a = vec![0u64; n];
        let mut b = vec![0u64; n];
        for i in 0..w {
            let (x, y) = if i < self.w {
                self.bit(i)
            } else if sign {
                msb
            } else {
                (0, 0)
            };
            a[(i / 64) as usize] |= x << (i % 64);
            b[(i / 64) as usize] |= y << (i % 64);
        }
        Const { w, signed: self.signed, a, b }
    }

    pub fn with_sign(mut self, s: bool) -> Const {
        self.signed = s;
        self
    }

    /// Fill literal ('0, '1, 'x, 'z) of width w.
    pub fn fill(d: char, w: u32) -> Const {
        let mut c = Const::zero(w);
        let (va, vb) = match d {
            '1' => (!0, 0),
            'x' => (!0, !0),
            'z' => (0, !0),
            _ => (0, 0),
        };
        for i in 0..c.a.len() {
            c.a[i] = va;
            c.b[i] = vb;
        }
        c.mask();
        c
    }

    pub fn from_number(n: &Number) -> Result<Const, String> {
        if n.fill {
            return Ok(Const::fill(n.digits.chars().next().unwrap(), 1));
        }
        let digits = &n.digits;
        if n.base == 10 {
            // x/z decimal
            if digits.chars().any(|c| c == 'x' || c == 'z') {
                let w = n.size.unwrap_or(32);
                let mut c = Const::fill(if digits.contains('x') { 'x' } else { 'z' }, w);
                c.signed = n.signed;
                return Ok(c);
            }
            // big decimal
            let mut words: Vec<u64> = vec![0];
            for ch in digits.chars() {
                let d = ch.to_digit(10).ok_or_else(|| format!("bad decimal digit '{ch}'"))? as u128;
                let mut carry = d;
                for w in words.iter_mut() {
                    let t = (*w as u128) * 10 + carry;
                    *w = t as u64;
                    carry = t >> 64;
                }
                if carry != 0 {
                    words.push(carry as u64);
                }
            }
            let needed = {
                let mut bl = 0;
                for i in (0..words.len()).rev() {
                    if words[i] != 0 {
                        bl = 64 * i as u32 + 64 - words[i].leading_zeros();
                        break;
                    }
                }
                bl
            };
            let w = n.size.unwrap_or(needed.max(32));
            let mut c = Const { w: w.max(1), signed: n.signed, a: words, b: vec![] };
            c.mask();
            return Ok(c);
        }
        let bits = match n.base {
            2 => 1,
            8 => 3,
            _ => 4,
        };
        let total = digits.len() as u32 * bits;
        let mut c = Const::zero(total.max(1));
        for (k, ch) in digits.chars().rev().enumerate() {
            let (va, vb) = match ch {
                'x' => ((1u64 << bits) - 1, (1u64 << bits) - 1),
                'z' => (0, (1u64 << bits) - 1),
                _ => (ch.to_digit(n.base).ok_or_else(|| format!("bad digit '{ch}' for base {}", n.base))? as u64, 0),
            };
            for i in 0..bits {
                let bit = k as u32 * bits + i;
                c.a[(bit / 64) as usize] |= ((va >> i) & 1) << (bit % 64);
                c.b[(bit / 64) as usize] |= ((vb >> i) & 1) << (bit % 64);
            }
        }
        let w = match n.size {
            Some(s) => s.max(1),
            None => total.max(32),
        };
        // extension: x/z msb extends as x/z, else zero
        let (ma, mb) = c.bit(c.w - 1);
        let ext_x = mb == 1;
        let mut r = c.resize(w, false);
        if ext_x && w > c.w {
            for i in c.w..w {
                r.a[(i / 64) as usize] |= ma << (i % 64);
                r.b[(i / 64) as usize] |= 1 << (i % 64);
            }
        }
        r.signed = n.signed;
        Ok(r)
    }
}
