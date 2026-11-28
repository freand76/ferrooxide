// exit: 1
// $bits takes a value of a bit-stream type (IEEE 1800-2017 20.6.2, 6.24.3), not a real.
module t;
  localparam w = $bits(1.0);
  initial $display(w);
endmodule
