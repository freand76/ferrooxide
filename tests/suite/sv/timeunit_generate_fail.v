// exit: 1
// timeunit belongs to a module, not a generate block (IEEE 1800-2017 3.14.2.2, 27.2).
module test #(parameter A = 1);
  if (A) begin : b
    timeunit 10ns / 1ns;
  end
endmodule
