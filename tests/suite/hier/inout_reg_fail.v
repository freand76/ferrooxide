// exit: 1
// An inout port is a net, never a reg (IEEE 1364-2005 12.3.3).
module test(x);
  inout reg x;
endmodule
