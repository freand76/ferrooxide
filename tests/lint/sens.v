// Sensitivity-list lint cases. Expected findings: tests/lint/sens.expected
module sub(input a, input b, output reg y);
  always @(a) y = a & b;                 // b missing (reported once for both instances)
  reg x;
endmodule

module tb;
  reg a, b, c, sel, clk, d, q;
  reg [3:0] v, s, idx;
  reg [7:0] mem [0:15];
  reg y1, y2, o, t, z, y3, y4, y5;
  reg [7:0] r;
  integer i;
  sub u1(a, b, ), u2(b, a, );
  function f(input p);
    f = p ^ c;
  endfunction
  always @(a or b) y2 = a & b;           // complete
  always @(sel or a)                     // b missing
    case (sel)
      1'b0: o = a;
      default: o = b;
    endcase
  always @(a or b) begin                 // t is a temporary: complete
    t = a ^ b;
    z = t & a;
  end
  always @(posedge clk) q <= d;          // edge triggered: not checked
  always @* y1 = a & b & c;              // @*: not checked
  always @(a) y3 = f(a);                 // c missing (read in the function)
  always @(idx) r = mem[idx];            // mem missing
  always @(v) begin                      // i is a loop variable: complete
    for (i = 0; i < 4; i = i + 1) s[i] = v[3 - i];
  end
  always @(a) y4 = u1.x | a;             // u1.x missing
  always @(a or b or c) y5 = a;          // extra entries are fine
endmodule
