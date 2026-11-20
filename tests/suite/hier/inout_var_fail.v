// exit: 1
// An inout port is a net, never a variable (IEEE 1800-2017 23.2.2.3).
module test(inout var logic x);
endmodule
