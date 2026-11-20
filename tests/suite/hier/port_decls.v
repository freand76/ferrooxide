// Non-ANSI port declarations completed by a net or variable declaration (IEEE 1364-2005
// 12.3.3, 1800-2017 23.2.2.1), in either order: the same range, or a data type with a port
// declaration without a range; also for task ports.
module m(a, b, c, d, e);
  input [7:0] a;
  wire [7:0] a;
  reg [3:0] b;
  output [3:0] b;
  output c;
  integer c;
  output d;
  real d;
  input e;
  wire e;
  initial begin
    b = 4'h9;
    c = -5;
    d = 2.5;
  end
endmodule

module test;
  wire [3:0] b;
  wire [31:0] c;
  wire d;
  reg [7:0] r;
  task t;
    input [7:0] x;
    reg [7:0] x;
    output y;
    integer y;
    begin
      y = x + 1;
    end
  endtask
  integer z;
  m u(8'h12, b, c, d, 1'b1);
  initial begin
    t(8'd41, z);
    #1;
    if (b === 4'h9 && $signed(c) === -5 && z === 42 && $bits(u.c) == 32) $display("PASSED");
    else $display("FAILED: b %h c %0d z %0d", b, $signed(c), z);
  end
endmodule
