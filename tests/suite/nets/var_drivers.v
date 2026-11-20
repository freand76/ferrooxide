// Variables with one driver each (IEEE 1800-2017 6.5): an output port written procedurally
// in its module, a variable driven through an output port, different bits of a variable
// driven by different continuous assignments.
module c1(output reg q, output logic [3:0] r);
  initial begin q = 1; r = 4'h5; end
endmodule
module c2(output int o);
  assign o = 7;
endmodule
module test;
  wire w;
  logic [3:0] v;
  int x;
  logic [7:0] parts;
  assign parts[3:0] = 4'h1;
  assign parts[7:4] = 4'h2;
  c1 a(w, v);
  c2 b(x);
  initial #1 if (w === 1 && v === 4'h5 && x === 7 && parts === 8'h21) $display("PASSED"); else $display("FAILED");
endmodule
