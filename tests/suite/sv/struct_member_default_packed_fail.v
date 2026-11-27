// exit: 1
// A member of a packed struct cannot have a default value (IEEE 1800-2017 7.2.2).
module m; struct packed { bit [3:0] a = 1; } x; endmodule
