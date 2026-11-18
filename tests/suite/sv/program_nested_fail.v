// exit: 1
// Programs cannot contain module declarations (IEEE 1800-2017 24.3).
program p;
  module m;
    initial $display("FAILED");
  endmodule
endprogram
