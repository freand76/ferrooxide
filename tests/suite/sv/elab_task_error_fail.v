// exit: 1
// An `$error` elaboration task that is elaborated makes elaboration fail (20.11).
module top #(parameter int N = 7) ();
  if (N > 5) $error("N = %0d is too big", N);
  initial $display("FAILED -- this ran");
endmodule
