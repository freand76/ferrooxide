// Enum values that fit the base type: the largest unsigned value, negative values of a
// signed base, and a full-width bit pattern of a signed base.
module test;
  enum bit [1:0] {a = 0, b = 3} u;
  enum bit signed [7:0] {n1 = -1, n2 = -128, p = 127} s;
  enum int {big = 'hFFFFFFFF} i;
  initial
    if (b === 2'd3 && n2 === -8'sd128 && big === -1)
      $display("PASSED");
    else
      $display("FAILED");
endmodule
