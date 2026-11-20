// exit: 1
// A replication count may not be negative.
module test;
  parameter wid = 9;
  reg [31:0] r;
  initial r = {(wid-16){8'b0}};
endmodule
