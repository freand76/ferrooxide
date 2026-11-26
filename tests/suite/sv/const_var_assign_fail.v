// exit: 1
// a continuous assignment to a const variable is an error (IEEE 1800-2017 6.20.6)
module top;
  const logic [3:0] k = 5;
  assign k[0] = 1;
endmodule
