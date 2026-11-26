// exit: 1
// the seed of a $dist_* function is an integral variable (IEEE 1800-2017 20.15)
module top;
  initial $display($dist_normal(5, 0, 10));
endmodule
