// exit: 1
// a uwire takes one driver (IEEE 1800-2017 6.6.2), a port connection counts
module drv(output logic o);
  assign o = 1;
endmodule
module top;
  uwire u;
  drv d(u);
  assign u = 0;
endmodule
