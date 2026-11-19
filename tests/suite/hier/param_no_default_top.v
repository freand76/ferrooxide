// A module with a parameter port that has no default is not a top-level module: only an
// instance can give the value (IEEE 1800-2017 6.20.1, 23.3.1).
module a #(parameter A);
  initial $display("FAILED");
endmodule

module test;
  initial $display("PASSED");
endmodule
