// exit: 1
// A struct that contains a union cannot give its members default values (IEEE 1800-2017 7.2.2).
module m; typedef union { int a; bit [31:0] b; } u_t; struct { u_t u; int k = 1; } x; endmodule
