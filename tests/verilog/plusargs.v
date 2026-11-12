// args: +run_ns=1500.25 +name=hello +n=42 +mask=ff +flag +empty=
// $test$plusargs / $value$plusargs with %f %d %h %s, and plusargs that are absent.
module tb;
  real t;
  integer n, m;
  reg [31:0] mask;
  reg [8*16-1:0] name;
  initial begin
    $display("flag=%b fl=%b nope=%b", $test$plusargs("flag"), $test$plusargs("fl"), $test$plusargs("nope"));
    if ($value$plusargs("run_ns=%f", t)) $display("run_ns=%f", t);
    if ($value$plusargs("n=%d", n)) $display("n=%0d", n);
    m = 5;
    if (!$value$plusargs("missing=%d", m)) $display("missing, m=%0d", m);
    if ($value$plusargs("mask=%h", mask)) $display("mask=%h", mask);
    if ($value$plusargs("name=%s", name)) $display("name=%0s", name);
    $display("empty=%b", $value$plusargs("empty=%d", m));
  end
endmodule
