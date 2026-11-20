// exit: 1
// The variable of a generate loop is a genvar (IEEE 1800-2017 27.4).
module test;
  integer i;
  for (i = 0; i < 2; i = i + 1) begin : g
    reg r;
  end
endmodule
