// exit: 1
// An unnamed block takes no end label (IEEE 1800-2017 9.3.4).
module test;
  initial fork
  join : x
endmodule
