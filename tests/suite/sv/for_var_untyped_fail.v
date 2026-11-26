// exit: 1
// a for loop variable declared with `var` names its data type (IEEE 1800-2017 A.6.8:
// `[var] data_type name = e`): `var [7:0] i` is an error
module test;
  initial for (var [7:0] i = 0; i < 10; i++) ;
endmodule
