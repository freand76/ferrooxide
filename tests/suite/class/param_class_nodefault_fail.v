// exit: 1
// A class with a parameter without a default exists only as specializations.
module test;
  class C #(int N);
  endclass
  C c;
endmodule
