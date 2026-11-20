// exit: 1
// A parameter cannot be assigned (IEEE 1800-2017 6.20), also an unpacked array one.
module t;
  localparam int A [2] = '{1, 2};
  initial A[0] = 3;
endmodule
