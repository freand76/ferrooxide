// interconnect nets (IEEE 1800-2017 6.6.8) connecting ports
module mi(input in, input [3:0] v, output logic ok);
  initial #1 ok = (in === 1'b1 && v === 4'ha);
endmodule
module mo(output out, output [3:0] v);
  assign out = 1;
  assign v = 4'ha;
endmodule
module top;
  interconnect bus;
  interconnect [3:0] vb;
  wire ok;
  mi a(bus, vb, ok);
  mo b(bus, vb);
  initial #2 if (ok) $display("PASSED"); else $display("FAILED");
endmodule
