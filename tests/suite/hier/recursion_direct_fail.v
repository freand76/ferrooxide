// exit: 1
// A module cannot instantiate itself outside a generate block.
module r ();
  r next();
endmodule

module test;
  r top();
endmodule
