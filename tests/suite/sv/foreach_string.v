// expected
// foreach over the characters of a string (as over a dynamic array of bytes), also of a string
// property; len(), getc() and indexing of a string property.
class C;
  string p = "xyz";
endclass
module t;
  function automatic bit has_star(string arg);
    foreach (arg[i]) if (arg[i] == "*") return 1;
    return 0;
  endfunction
  string s = "abc";
  C c = new;
  initial begin
    foreach (s[i]) $display("%0d %s", i, s[i]);
    $display("%0d %0d", has_star("a*b"), has_star("ab"));
    $display("%0d %0d %s %0d", c.p.len(), c.p.getc(1), c.p[1], c.p[2]);
    foreach (c.p[i]) $display("p %0d %s", i, c.p[i]);
  end
endmodule
