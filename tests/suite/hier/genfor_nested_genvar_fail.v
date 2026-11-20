// exit: 1
// A generate loop may not use the genvar of an enclosing loop (IEEE 1800-2017 27.4).
module test;
  genvar i;
  for (i = 0; i < 2; i = i + 1) begin : a
    for (i = 0; i < 2; i = i + 1) begin : b
      reg r;
    end
  end
endmodule
