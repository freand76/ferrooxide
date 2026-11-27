// expected
// Default member values of unpacked structs (IEEE 1800-2017 7.2.2): variables without an
// initializer start with them (other members with their type's start value), an
// initializer replaces them; module, block, automatic, array, nested and class variables.
module top;
  parameter c = 4'h5;
  typedef struct { bit [3:0] lo = c; bit [3:0] hi; logic [1:0] l; int i = 7; real r = 1.5; string s = "hi"; } s_t;
  typedef struct { s_t in; int k = 3; logic m; } o_t;
  s_t a;
  s_t b = '{lo: 1, hi: 2, l: 0, i: 0, r: 0.0, s: ""};
  s_t arr [2];
  o_t o;
  class C; s_t p; endclass
  task automatic t; s_t v; $display("%h %0d %s", v.lo, v.i, v.s); endtask
  initial begin
    C h = new;
    s_t loc;
    $display("%h %h %b %0d %f %s", a.lo, a.hi, a.l, a.i, a.r, a.s);
    $display("%h %h", b.lo, b.i);
    $display("%h %0d %h", arr[1].lo, arr[0].i, arr[0].hi);
    $display("%h %0d %0d %b %b", o.in.lo, o.in.i, o.k, o.m, o.in.l);
    $display("%h %0d", h.p.lo, h.p.i);
    $display("%h", loc.lo);
    t;
  end
endmodule
