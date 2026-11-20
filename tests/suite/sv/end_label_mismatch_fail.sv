// exit: 1
// An end label must repeat the block's name (IEEE 1800-2017 9.3.4).
module test;
  initial begin : a
  end : b
endmodule
