// Elaboration system tasks (IEEE 1800-2017 20.11): reported when their generate scope is
// elaborated (on stderr); `$info` and `$warning` let the simulation run, those in branches
// that are not elaborated say nothing.
module m #(parameter int N = 3) ();
  if (N > 2) begin : g
    $info("N is %0d", N);
    $warning("big N");
  end
  if (N > 5) $error("too big");
  else if (N < 0) $fatal(1, "negative");
  initial if (N == 4) $display("PASSED");
endmodule
module top;
  m #(4) a();
endmodule
