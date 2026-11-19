// expected
// A source file that is not UTF-8 (a Latin-1 comment: é, ÿ) is read.
module m;
  // café
  initial $display("read");
endmodule
