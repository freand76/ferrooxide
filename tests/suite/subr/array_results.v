// Functions with an unpacked array result (IEEE 1800-2017 13.4.1), also in a parameter
// value, and parameters whose typedef gives the unpacked dimensions.
module t;
  typedef logic [9:0] exp_t [0:7];
  typedef int pair_t [2];

  function automatic exp_t build_exp(int base);
    exp_t t;
    for (int i = 0; i < 8; ++i) t[i] = base + i;
    return t;
  endfunction

  // the result variable named by the function
  function pair_t swap(pair_t p);
    swap[0] = p[1];
    swap[1] = p[0];
  endfunction

  localparam exp_t EXP = build_exp(1);
  localparam exp_t LIT = '{8{10'd5}};
  localparam pair_t P = swap('{3, 4});

  class C;
    pair_t q;
    function void set(); q = swap('{7, 8}); endfunction
  endclass

  exp_t v;
  pair_t p;
  C c;
  bit ok = 1;

  initial begin
    foreach (EXP[i]) if (EXP[i] != i + 1) ok = 0;
    foreach (LIT[i]) if (LIT[i] != 5) ok = 0;
    if (P[0] != 4 || P[1] != 3) ok = 0;
    v = build_exp(20);
    if (v[0] != 20 || v[7] != 27) ok = 0;
    p = swap(swap('{1, 2}));
    if (p[0] != 1 || p[1] != 2) ok = 0;
    p = swap(p);
    if (p[0] != 2 || p[1] != 1) ok = 0;
    c = new;
    c.set();
    if (c.q[0] != 8 || c.q[1] != 7) ok = 0;
    if (ok) $display("PASSED");
    else $display("FAILED");
  end
endmodule
