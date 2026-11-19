// A parameter port may leave out its default; every instance then overrides it, by name or
// by position (IEEE 1800-2017 6.20.1, 23.10).
module m #(parameter W, parameter int N, parameter bit [1:0][3:0] K) (output [W-1:0] o);
  assign o = N + K[1];
endmodule

module test;
  wire [3:0] a;
  wire [7:0] b;
  m #(.W(4), .N(5), .K(8'h21)) u1(a);
  m #(8, 200, 8'h3f) u2(b);
  initial begin
    #1;
    if (a === 4'd7 && b === 8'd203 && $bits(b) == 8) $display("PASSED");
    else $display("FAILED");
  end
endmodule
