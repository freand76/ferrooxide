// exit: 1
// A parameter value may not use a specparam (IEEE 1800-2017 6.20.5).
module test;
  specparam sp = 2;
  localparam lp = {sp{1'b1}};
endmodule
