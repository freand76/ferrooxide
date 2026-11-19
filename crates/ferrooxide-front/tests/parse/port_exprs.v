// Port expressions (IEEE 1364-2005 12.3.1): a port that is a part select or a concatenation
// of the module's signals, named (`.p(expr)`) or connected by position; one signal may
// appear in several ports. Unconnected bits of the signals stay z. An expression port
// without a name is connected by position only.
module sel(a[7:4], .lo(a[1:0]), b[3:0]);
  input [7:0] a;
  output [7:0] b;
  assign b = {a[7:4] + 4'd1, a[3:0]};
endmodule

module cat({x, y[2:1]}, .o({p, q}));
  input [1:0] x;
  input [3:0] y;
  output p;
  output [2:0] q;
  assign {p, q} = {x, y[2:1]};
endmodule

module test;
  reg [3:0] hi = 4'h9;
  reg [1:0] lo = 2'b10;
  wire [3:0] b;
  reg [3:0] in = 4'b1011;
  wire [3:0] o;
  int fails = 0;
  sel s(hi, lo, b);
  sel s2(.lo(lo));
  cat c(in, o);
  initial begin
    #1;
    if (s.a !== 8'b1001zz10) begin $display("FAILED a=%b", s.a); fails++; end
    if (b !== 4'bzz10 || s2.a !== 8'bzzzzzz10) begin $display("FAILED b=%b", b); fails++; end
    if (o !== 4'b1011 || c.y !== 4'bz11z) begin $display("FAILED o=%b y=%b", o, c.y); fails++; end
    hi = 4'h3;
    lo = 2'b01;
    #1;
    if (s.a !== 8'b0011zz01 || b !== 4'bzz01) begin $display("FAILED a=%b b=%b", s.a, b); fails++; end
    if (fails == 0) $display("PASSED");
  end
endmodule
