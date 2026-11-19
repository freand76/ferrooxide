// exit: 1
// A positional assignment pattern needs one item per array element (IEEE 1800-2017 10.9.1).
module test;
  int a[4];
  initial a = '{1, 2, 3};
endmodule
