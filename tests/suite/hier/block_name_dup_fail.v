// exit: 1
// A named block cannot take the name of a generate block of the same scope.
module test;
  if (1) begin : b
    wire w;
  end
  initial begin : b
    $display("FAILED");
  end
endmodule
