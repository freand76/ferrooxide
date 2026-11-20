// exit: 1
// Two generate blocks with one name in a scope (IEEE 1800-2017 27.6).
module test;
  for (genvar i = 0; i < 1; i = i + 1) begin : g
  end
  for (genvar i = 0; i < 1; i = i + 1) begin : g
  end
endmodule
