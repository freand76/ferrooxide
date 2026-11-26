// user-defined nettypes without a resolution function (IEEE 1800-2017 6.6.7): nets of
// their data type with one driver; uwire (6.6.2) likewise
module drv(output logic [7:0] o);
  assign o = 8'h5a;
endmodule

module top;
  typedef struct packed { logic [3:0] hi, lo; } pair_t;
  nettype logic [7:0] byte_net;
  nettype real real_net;
  nettype pair_t pair_net;
  byte_net b, p;
  real_net r;
  pair_net q;
  uwire [3:0] u;
  assign b = 8'h33;
  assign r = 2.5;
  assign q = '{hi: 4'h1, lo: 4'h2};
  assign u = 4'h9;
  drv d(p);
  initial begin
    #1;
    if (b === 8'h33 && r == 2.5 && q.hi == 1 && q.lo == 2 && $bits(q) == 8 && u === 4'h9 && p === 8'h5a)
      $display("PASSED");
    else
      $display("FAILED %h %f %h %h %h", b, r, q, u, p);
  end
endmodule
