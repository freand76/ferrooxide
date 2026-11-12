// Nets with several drivers: a tri-state bus shared through inout ports, byte-lane drivers,
// conflicts (x), nothing driving (z), and a driver reading the bus back.
module dev #(parameter [7:0] ID = 0) (input oe, input [7:0] val, inout [7:0] bus, output [7:0] seen);
  assign bus = oe ? val ^ ID : 8'bz;
  assign seen = bus;
endmodule

module lanes(input [1:0] oe, input [15:0] v, inout [15:0] q);
  assign q[15:8] = oe[1] ? v[15:8] : 8'bz;
  assign q[7:0]  = oe[0] ? v[7:0]  : 8'bz;
endmodule

module tb;
  reg oe1 = 0, oe2 = 0;
  reg [7:0] v1 = 8'h0f, v2 = 8'hf0;
  wire [7:0] bus, s1, s2;
  dev #(8'h00) d1(oe1, v1, bus, s1);
  dev #(8'h01) d2(oe2, v2, bus, s2);
  reg [1:0] loe = 0;
  reg [15:0] lv = 16'hbeef;
  wire [15:0] q;
  lanes l(loe, lv, q);
  assign q = loe == 2'b00 ? 16'h1234 : 16'hzzzz;
  wire pull;
  assign pull = 1'b1;
  assign pull = 1'bz;
  initial begin
    #1 $display("none : bus=%h s1=%h s2=%h q=%h pull=%b", bus, s1, s2, q, pull);
    oe1 = 1;
    #1 $display("d1   : bus=%h s1=%h s2=%h", bus, s1, s2);
    oe2 = 1;
    #1 $display("both : bus=%b", bus);
    v2 = 8'h0e;
    #1 $display("same : bus=%b", bus);
    oe1 = 0;
    #1 $display("d2   : bus=%h s1=%h", bus, s1);
    loe = 2'b01;
    #1 $display("lane0: q=%h", q);
    loe = 2'b11;
    #1 $display("both : q=%h", q);
    loe = 2'b10; lv = 16'h1200;
    #1 $display("lane1: q=%h", q);
  end
endmodule
