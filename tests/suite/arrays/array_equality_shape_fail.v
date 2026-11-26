// exit: 1
// arrays compared have the same shape (IEEE 1800-2017 7.6)
module top;
  int a [4], b [5];
  initial $display(a == b);
endmodule
