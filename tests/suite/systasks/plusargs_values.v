// args: +hex=12_ab_x_z +bin=1_0z +dec=1_000 +neg=-1_0 +decx=x +bad=nope +r=2.5units +name=abc
// $test$plusargs and $value$plusargs with the prefix or format in a variable, and the
// values of the plusargs (IEEE 1800-2017 21.6): `_` separators, x and z digits, a leading
// minus, an invalid number (x), a real followed by other characters, text into a string
// variable (also a method's argument).
module m;
  class C;
    function string get(string name = "");
      void'($value$plusargs("name=%s", name));
      return name;
    endfunction
  endclass
  C c;
  string str;
  reg [8*8:1] f;
  string sf;
  reg [23:0] h;
  reg [3:0] b;
  integer d;
  reg [7:0] n8;
  real r;
  int errors = 0;
  initial begin
    f = "hex=%h";
    if (!$value$plusargs(f, h) || h !== 24'h12abxz) begin errors++; $display("FAILED -- hex %h", h); end
    sf = "bin=%b";
    if (!$value$plusargs(sf, b) || b !== 4'b010z) begin errors++; $display("FAILED -- bin %b", b); end
    if (!$value$plusargs("dec=%d", d) || d !== 1000) begin errors++; $display("FAILED -- dec %0d", d); end
    if (!$value$plusargs("neg=%d", d) || d !== -10) begin errors++; $display("FAILED -- neg %0d", d); end
    if (!$value$plusargs("neg=%h", n8) || n8 !== 8'hf0) begin errors++; $display("FAILED -- neg hex %h", n8); end
    if (!$value$plusargs("decx=%d", d) || d !== 32'bx) begin errors++; $display("FAILED -- decx %0d", d); end
    if (!$value$plusargs("bad=%d", d) || d !== 32'bx) begin errors++; $display("FAILED -- bad %0d", d); end
    if (!$value$plusargs("r=%f", r) || r != 2.5) begin errors++; $display("FAILED -- real %f", r); end
    if (!$value$plusargs("dec=%d", r) || r != 1000.0) begin errors++; $display("FAILED -- dec real %f", r); end
    if ($value$plusargs("missing=%d", d)) begin errors++; $display("FAILED -- missing"); end
    if (!$value$plusargs("name=%s", str) || str != "abc" || str.len() != 3) begin errors++; $display("FAILED -- string '%s'", str); end
    c = new;
    if (c.get() != "abc") begin errors++; $display("FAILED -- string argument"); end
    sf = "name";
    if (!$test$plusargs(sf) || $test$plusargs("nam_")) begin errors++; $display("FAILED -- test$plusargs"); end
    if (errors == 0) $display("PASSED");
  end
endmodule
