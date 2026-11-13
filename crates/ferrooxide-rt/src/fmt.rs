//! $display-style formatting, compatible with Icarus Verilog output.

use crate::{nw, FmtArg};
use std::cell::RefCell;

/// `$timeformat` settings: (units exponent, precision, suffix, minimum width), and the
/// simulation precision exponent. None: %t prints simulation ticks.
#[derive(Clone, Debug)]
pub struct TimeFormat {
    pub units: i32,
    pub prec: usize,
    pub suffix: String,
    pub width: usize,
}

thread_local! {
    static TIME_FORMAT: RefCell<(Option<TimeFormat>, i32)> = const { RefCell::new((None, 0)) };
}

/// Set by the simulator: the simulation precision (power of ten of seconds).
pub fn set_precision(p: i32) {
    TIME_FORMAT.with(|t| t.borrow_mut().1 = p);
}

/// `$timeformat` (None: the default).
pub fn set_time_format(f: Option<TimeFormat>) {
    TIME_FORMAT.with(|t| t.borrow_mut().0 = f);
}

/// %t of a time in simulation ticks, with the `$timeformat` settings if any.
fn fmt_time(ticks: f64, int: Option<String>, pad: bool) -> String {
    TIME_FORMAT.with(|t| {
        let t = t.borrow();
        match &t.0 {
            Some(f) => {
                let v = ticks * 10f64.powi(t.1 - f.units);
                let s = format!("{:.*}{}", f.prec, v, f.suffix);
                if pad {
                    format!("{:>w$}", s, w = f.width)
                } else {
                    s
                }
            }
            None => {
                let s = int.unwrap_or_else(|| format!("{}", ticks as i128));
                if pad {
                    format!("{:>20}", s)
                } else {
                    s
                }
            }
        }
    })
}

/// A value view: planes (aval, bval) as word vectors.
pub struct Val<'a> {
    pub a: &'a [u64],
    pub b: &'a [u64],
    pub w: u32,
    pub signed: bool,
}

impl Val<'_> {
    fn bit(&self, i: u32) -> (u64, u64) {
        let (wi, bi) = ((i / 64) as usize, i % 64);
        ((self.a[wi] >> bi) & 1, (self.b[wi] >> bi) & 1)
    }
    fn known(&self) -> bool {
        self.b.iter().all(|&x| x == 0)
    }
}

/// Decimal string of an unsigned multi-word number.
pub fn to_dec_unsigned(words: &[u64]) -> String {
    if words.len() <= 2 {
        let v = words[0] as u128 | ((*words.get(1).unwrap_or(&0) as u128) << 64);
        return v.to_string();
    }
    let mut w = words.to_vec();
    let mut digits = Vec::new();
    loop {
        // divide by 10^18
        let mut rem: u128 = 0;
        for x in w.iter_mut().rev() {
            let cur = (rem << 64) | *x as u128;
            *x = (cur / 1_000_000_000_000_000_000) as u64;
            rem = cur % 1_000_000_000_000_000_000;
        }
        let zero = w.iter().all(|&x| x == 0);
        if zero {
            digits.push(format!("{}", rem));
            break;
        }
        digits.push(format!("{:018}", rem));
    }
    digits.reverse();
    digits.concat()
}

fn dec_digits_for_width(w: u32, signed: bool) -> usize {
    // number of decimal digits of 2^w - 1 (unsigned) or 2^(w-1) plus a sign
    if w == 0 {
        return 1;
    }
    let bits = if signed { w - 1 } else { w };
    let d = ((bits as f64) * std::f64::consts::LOG10_2).floor() as usize + 1;
    if signed {
        d + 1
    } else {
        d
    }
}

pub fn fmt_dec(v: &Val, pad: bool) -> String {
    let width = if pad { dec_digits_for_width(v.w, v.signed) } else { 0 };
    let s = if !v.known() {
        let n = nw(v.w);
        let mut allx = true;
        let mut allz = true;
        let mut anyx = false;
        let mut anyz = false;
        for i in 0..v.w {
            let (a, b) = v.bit(i);
            let isx = b == 1 && a == 1;
            let isz = b == 1 && a == 0;
            allx &= isx;
            allz &= isz;
            anyx |= isx;
            anyz |= isz;
        }
        let _ = n;
        if allx {
            "x".into()
        } else if allz {
            "z".into()
        } else if anyx {
            "X".into()
        } else if anyz {
            "Z".into()
        } else {
            "x".into()
        }
    } else {
        let neg = v.signed && v.w > 0 && v.bit(v.w - 1).0 == 1;
        if neg {
            // two's complement magnitude
            let n = nw(v.w);
            let mut mag: Vec<u64> = v.a.iter().map(|x| !x).collect();
            let r = v.w % 64;
            if r != 0 {
                mag[n - 1] &= (1u64 << r) - 1;
            }
            // +1
            for x in mag.iter_mut() {
                let (s, o) = x.overflowing_add(1);
                *x = s;
                if !o {
                    break;
                }
            }
            if r != 0 {
                mag[n - 1] &= (1u64 << r) - 1;
            }
            format!("-{}", to_dec_unsigned(&mag))
        } else {
            to_dec_unsigned(v.a)
        }
    };
    format!("{:>width$}", s, width = width)
}

pub fn fmt_radix(v: &Val, bits_per: u32, pad: bool) -> String {
    let ndig = v.w.div_ceil(bits_per).max(1);
    let mut out = Vec::with_capacity(ndig as usize);
    for dgt in (0..ndig).rev() {
        let lo = dgt * bits_per;
        let hi = (lo + bits_per).min(v.w);
        let (mut val, mut nx, mut nz) = (0u32, 0, 0);
        for i in lo..hi {
            let (a, b) = v.bit(i);
            if b == 1 {
                if a == 1 {
                    nx += 1
                } else {
                    nz += 1
                }
            }
            val |= (a as u32) << (i - lo);
        }
        let n = hi - lo;
        let c = if nx == n {
            'x'
        } else if nz == n {
            'z'
        } else if nx > 0 {
            'X'
        } else if nz > 0 {
            'Z'
        } else {
            std::char::from_digit(val, 16).unwrap()
        };
        out.push(c);
    }
    if !pad {
        // strip leading zeros
        let first = out.iter().position(|&c| c != '0').unwrap_or(out.len() - 1);
        out.drain(..first);
    }
    out.into_iter().collect()
}

fn fmt_str(v: &Val) -> String {
    let mut s = String::new();
    let nbytes = v.w.div_ceil(8);
    for k in (0..nbytes).rev() {
        let mut byte = 0u32;
        for i in 0..8 {
            let bit = k * 8 + i;
            if bit < v.w {
                // x/z bits count as 0 (as in Icarus)
                let (a, b) = v.bit(bit);
                byte |= ((a & !b & 1) as u32) << i;
            }
        }
        if byte != 0 {
            s.push(char::from_u32(byte).unwrap_or('?'));
        }
    }
    s
}

/// %s: one character per byte, NUL (or unknown) bytes as spaces; without padding (%0s) the
/// leading NUL bytes are dropped. As Icarus does it.
fn fmt_disp_str(v: &Val, pad: bool) -> String {
    let mut s = String::new();
    let nbytes = v.w.div_ceil(8);
    for k in (0..nbytes).rev() {
        let mut byte = 0u32;
        for i in 0..8 {
            let bit = k * 8 + i;
            if bit < v.w {
                let (a, b) = v.bit(bit);
                byte |= ((a & !b & 1) as u32) << i;
            }
        }
        if byte == 0 {
            if pad || !s.is_empty() {
                s.push(' ');
            }
        } else {
            s.push(char::from_u32(byte).unwrap_or('?'));
        }
    }
    s
}

fn fmt_char(v: &Val) -> String {
    let mut byte = 0u32;
    for i in 0..8.min(v.w) {
        byte |= (v.bit(i).0 as u32) << i;
    }
    char::from_u32(byte).map(|c| c.to_string()).unwrap_or_default()
}

/// Format the argument list of a $display-family call.
/// `get(slot, w)` returns the planes of a value.
/// `texp`: %t prints a time in module units multiplied by 10^texp (simulation ticks).
pub fn format_args<'m>(args: &[FmtArg], mem: &'m [u64], scope: &str, default_radix: char, texp: u8) -> String {
    let mut out = String::new();
    let view = |s: u32, w: u32, signed: bool| {
        let n = nw(w);
        let s = s as usize;
        Val { a: &mem[s..s + n], b: &mem[s + n..s + 2 * n], w, signed }
    };
    let mut i = 0;
    while i < args.len() {
        match &args[i] {
            FmtArg::Lit(f) => {
                i += 1;
                let chars: Vec<char> = f.chars().collect();
                let mut k = 0;
                while k < chars.len() {
                    let c = chars[k];
                    if c != '%' {
                        out.push(c);
                        k += 1;
                        continue;
                    }
                    k += 1;
                    if k >= chars.len() {
                        break;
                    }
                    // flags / width
                    let mut left = false;
                    if chars[k] == '-' {
                        left = true;
                        k += 1;
                    }
                    let mut width_s = String::new();
                    while k < chars.len() && chars[k].is_ascii_digit() {
                        width_s.push(chars[k]);
                        k += 1;
                    }
                    let mut prec: Option<usize> = None;
                    if k < chars.len() && chars[k] == '.' {
                        k += 1;
                        let mut p = String::new();
                        while k < chars.len() && chars[k].is_ascii_digit() {
                            p.push(chars[k]);
                            k += 1;
                        }
                        prec = Some(p.parse().unwrap_or(0));
                    }
                    if k >= chars.len() {
                        break;
                    }
                    let upper = matches!(chars[k], 'E' | 'G');
                    let spec = chars[k].to_ascii_lowercase();
                    k += 1;
                    if spec == '%' {
                        out.push('%');
                        continue;
                    }
                    if spec == 'm' {
                        out.push_str(scope);
                        continue;
                    }
                    let explicit_w: Option<usize> = width_s.parse().ok();
                    let pad = explicit_w.is_none();
                    let arg = args.get(i);
                    i += 1;
                    let s = match arg {
                        Some(FmtArg::Real { s }) => {
                            let r = f64::from_bits(mem[*s as usize]);
                            match spec {
                                'f' | 'e' | 'g' => {
                                    let t = fmt_real(r, spec, prec.unwrap_or(6));
                                    if upper {
                                        t.to_uppercase()
                                    } else {
                                        t
                                    }
                                }
                                // an infinite (or NaN) real prints as such also with %d
                                'd' if !r.is_finite() => fmt_real(r, 'f', 0),
                                't' => {
                                    let t = r * 10f64.powi(texp as i32);
                                    fmt_time(t, Some(format!("{}", t.round() as i128)), pad)
                                }
                                's' | 'c' => String::new(),
                                'b' | 'o' | 'h' | 'x' => {
                                    // rounded, as a 64-bit two's complement integer
                                    let i = [r.round() as i64 as u64];
                                    let v = Val { a: &i, b: &[0], w: 64, signed: false };
                                    let bits = match spec {
                                        'b' => 1,
                                        'o' => 3,
                                        _ => 4,
                                    };
                                    fmt_radix(&v, bits, pad)
                                }
                                _ => {
                                    let v = r.round();
                                    if v == 0.0 && r < 0.0 {
                                        "-0".to_string()
                                    } else {
                                        format!("{}", v as i128)
                                    }
                                }
                            }
                        }
                        Some(FmtArg::Val { .. }) | Some(FmtArg::Time { .. }) => {
                            let (s, w, signed) = match arg {
                                Some(FmtArg::Val { s, w, signed }) => (*s, *w, *signed),
                                Some(FmtArg::Time { s }) => (*s, 64, false),
                                _ => unreachable!(),
                            };
                            let v = view(s, w, signed);
                            let _ = s;
                            match spec {
                                'd' => fmt_dec(&v, pad),
                                'b' => fmt_radix(&v, 1, pad),
                                'o' => fmt_radix(&v, 3, pad),
                                'h' | 'x' => fmt_radix(&v, 4, pad),
                                'c' => fmt_char(&v),
                                's' => fmt_disp_str(&v, pad),
                                't' => {
                                    let s = scaled_time(&v, texp);
                                    if v.known() && v.w <= 64 {
                                        let t = v.a[0] as f64 * 10f64.powi(texp as i32);
                                        fmt_time(t, Some(s), pad)
                                    } else if pad {
                                        format!("{:>20}", s)
                                    } else {
                                        s
                                    }
                                }
                                'e' | 'f' | 'g' => {
                                    let n = nw(w);
                                    let r = crate::ops::int_to_real(&mem[s as usize..s as usize + 2 * n], 0, w, signed);
                                    let t = fmt_real(r, spec, prec.unwrap_or(6));
                                    if upper {
                                        t.to_uppercase()
                                    } else {
                                        t
                                    }
                                }
                                'v' => {
                                    let (a, b) = v.bit(0);
                                    match (a, b) {
                                        (0, 0) => "St0".into(),
                                        (1, 0) => "St1".into(),
                                        (0, 1) => "HiZ".into(),
                                        _ => "StX".into(),
                                    }
                                }
                                _ => fmt_dec(&v, pad),
                            }
                        }
                        Some(FmtArg::Lit(l)) => l.clone(),
                        _ => String::new(),
                    };
                    match explicit_w {
                        Some(wd) if wd > 0 => {
                            if left {
                                out.push_str(&format!("{:<wd$}", s));
                            } else if matches!(spec, 'b' | 'h' | 'x' | 'o') {
                                out.push_str(&format!("{:0>wd$}", s));
                            } else {
                                out.push_str(&format!("{:>wd$}", s));
                            }
                        }
                        _ => out.push_str(&s),
                    }
                }
            }
            FmtArg::Val { s, w, signed } => {
                let v = view(*s, *w, *signed);
                let s = match default_radix {
                    'b' => fmt_radix(&v, 1, true),
                    'h' => fmt_radix(&v, 4, true),
                    'o' => fmt_radix(&v, 3, true),
                    _ => fmt_dec(&v, true),
                };
                out.push_str(&s);
                i += 1;
            }
            FmtArg::Time { s } => {
                let v = view(*s, 64, false);
                out.push_str(&format!("{:>20}", fmt_dec(&v, false)));
                i += 1;
            }
            FmtArg::Real { s } => {
                out.push_str(&fmt_real(f64::from_bits(mem[*s as usize]), 'g', 6));
                i += 1;
            }
            FmtArg::Empty => {
                out.push(' ');
                i += 1;
            }
        }
    }
    out
}

/// A time value in module units scaled to simulation ticks (decimal, x/z as for %d).
fn scaled_time(v: &Val, texp: u8) -> String {
    if texp == 0 || !v.known() || v.w > 64 {
        return fmt_dec(v, false);
    }
    let t = v.a[0] as u128 * 10u128.pow(texp as u32);
    t.to_string()
}

/// C-style %f / %e / %g
pub fn fmt_real(v: f64, spec: char, prec: usize) -> String {
    let fmt_e = |v: f64, prec: usize| -> String {
        let s = format!("{:.*e}", prec, v);
        let (mant, exp) = s.split_once('e').unwrap();
        let e: i32 = exp.parse().unwrap();
        format!("{}e{}{:02}", mant, if e < 0 { '-' } else { '+' }, e.abs())
    };
    if !v.is_finite() {
        return if v.is_nan() { "nan".into() } else if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    match spec {
        'f' => format!("{:.*}", prec, v),
        'e' => fmt_e(v, prec),
        _ => {
            let p = if prec == 0 { 1 } else { prec };
            if v == 0.0 {
                return "0".into();
            }
            let e: i32 = {
                let s = format!("{:.*e}", p - 1, v);
                s.split_once('e').unwrap().1.parse().unwrap()
            };
            let s = if e < -4 || e >= p as i32 {
                fmt_e(v, p - 1)
            } else {
                format!("{:.*}", (p as i32 - 1 - e).max(0) as usize, v)
            };
            // strip trailing zeros of the fraction
            if let Some((m, ex)) = s.split_once('e') {
                let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
                format!("{m}e{ex}")
            } else if s.contains('.') {
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                s
            }
        }
    }
}

/// The characters of a value (as $display %s, leading zero bytes dropped).
pub fn value_string(mem: &[u64], s: usize, w: u32) -> String {
    let n = nw(w);
    let v = Val { a: &mem[s..s + n], b: &mem[s + n..s + 2 * n], w, signed: false };
    fmt_str(&v)
}
