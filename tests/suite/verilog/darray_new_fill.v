// expected
// new[n](v) with a single real or string as the initializer: every element is v (the
// reference accepts it; the standard wants an array). An array initializer still copies.
module test;
  real r[];
  string s[];
  int d[], e[];
  initial begin
    r = new[3](1.5);
    s = new[2]("A");
    d = '{7, 7, 7, 7};
    e = new[3](d);
    $display("%0d %f %f %0d %s %s", r.size(), r[0], r[2], s.size(), s[0], s[1]);
    $display("%0d %0d %0d %0d", d[0], d[3], e.size(), e[2]);
    e = new[5]('{1, 2});
    $display("%0d %0d %0d", e[1], e[2], e.size());
  end
endmodule
