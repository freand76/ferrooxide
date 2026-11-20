// exit: 1
// A port declaration with a data type declares the port completely.
module m(x);
  output integer x;
  integer x;
endmodule
module test;
  m u();
endmodule
