// exit: 1
// A generate condition may not use a specparam (IEEE 1800-2017 6.20.5).
module test;
  specparam sp = 1;
  if (sp) begin : b
  end
endmodule
