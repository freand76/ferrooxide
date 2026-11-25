// expected
// foreach over arrays reached through objects and scopes (IEEE 1800-2017 12.7.3): a fixed-size
// array property (also descending, and through an array of handles), a queue property, an
// associative array property, a static property through the class scope, `this.` in a method.
class N;
  int m[4:2];
  int q[$];
  int aa[string];
  static int st[3] = '{7, 8, 9};
  int g[2][3];
  function void show();
    foreach (this.m[i]) $display("this.m[%0d]=%0d", i, m[i]);
    foreach (q[i]) $display("q[%0d]=%0d", i, q[i]);
  endfunction
endclass
module t;
  N ns[2];
  initial begin
    ns[0] = new; ns[1] = new;
    ns[1].m[3] = 9; ns[1].q = '{5, 6}; ns[0].aa["x"] = 1; ns[0].aa["a"] = 2;
    ns[1].g[1][2] = 12;
    foreach (ns[s]) foreach (ns[s].m[j]) $display("%0d %0d %0d", s, j, ns[s].m[j]);
    foreach (ns[s]) foreach (ns[s].q[j]) $display("q %0d %0d %0d", s, j, ns[s].q[j]);
    foreach (ns[0].aa[k]) $display("aa %s %0d", k, ns[0].aa[k]);
    foreach (N::st[i]) $display("st %0d %0d", i, N::st[i]);
    foreach (ns[1].g[i, j]) if (ns[1].g[i][j]) $display("g %0d %0d", i, j);
    foreach (ns[1].g[, j]) $display("g col %0d", j);
    ns[1].show();
  end
endmodule
