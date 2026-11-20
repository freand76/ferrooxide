// exit: 1
// A generate block may not declare a specparam (IEEE 1800-2017 27.2).
module test #(parameter A = 1);
  if (A) begin : b
    specparam x = 10;
  end
endmodule
