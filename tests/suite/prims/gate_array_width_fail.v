// exit: 1
// a terminal of an array of gates has one bit per instance or one bit for all of them
// (IEEE 1364-2005 7.1.6): 2 bits for 4 instances is an error
module top;
  reg [1:0] in;
  wire [3:0] out;
  buf b[3:0] (out, in);
endmodule
