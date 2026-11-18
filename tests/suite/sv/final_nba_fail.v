// exit: 1
// A final procedure cannot contain nonblocking assignments (IEEE 1800-2017 9.2.3).
module m;
  reg r;
  final r <= 1;
endmodule
