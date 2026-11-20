// exit: 1
// Queues assigned to each other need element types of the same signedness
// (IEEE 1800-2017 6.22.2, 7.6).
module test;
  logic [7:0] q1[$];
  logic signed [7:0] q2[$];
  initial q1 = q2;
endmodule
