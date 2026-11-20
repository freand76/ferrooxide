// A specparam may use another specparam and parameters, and a delay may use it.
module test;
  parameter P = 3;
  specparam s1 = P + 1;
  specparam s2 = s1 * 2;
  reg r = 0;
  initial begin
    #s2 r = 1;
    if ($time == 8 && r) $display("PASSED");
    else $display("FAILED");
  end
endmodule
