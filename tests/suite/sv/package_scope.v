// Package and file-scope ($unit) names are found through the scopes that declare them
// (IEEE 1800 26.3): parameters (with their declared range), enum constants, functions,
// packages importing packages; a module's own declaration hides an imported name.
localparam int UNIT_W = 6;
typedef enum logic [1:0] { U0, U1, U2 } unit_e;

package p;
  parameter [7:4] HI = 4'b1010;
  localparam int W = 8;
  typedef enum logic [2:0] { RED = 3'd1, GREEN = 3'd2, BLUE = 3'd4 } color_e;
  function automatic int twice(int x);
    return 2 * x + W - 8;
  endfunction
endpackage

package q;
  import p::*;
  localparam int W2 = W * 2;
endpackage

module hide;
  import p::*;
  localparam int W = 3;   // hides p::W
  logic [W-1:0] v;
endmodule

module test;
  import p::*;
  import q::W2;
  logic [W-1:0] a;
  logic [UNIT_W-1:0] b;
  color_e c;
  unit_e u;
  reg failed = 0;
  hide h ();
  initial begin
    if ($bits(a) != 8 || $bits(b) != 6 || W2 != 16) failed = 1;
    if (HI[7] !== 1'b1 || HI[4] !== 1'b0 || HI[6:5] !== 2'b01) failed = 1;
    c = GREEN;
    if (c != 3'd2 || BLUE != 3'd4 || p::RED != 3'd1) failed = 1;
    u = U2;
    if (u != 2'd2) failed = 1;
    if (twice(5) != 10 || p::twice(3) != 6) failed = 1;
    if ($bits(h.v) != 3) failed = 1;
    if (failed) $display("FAILED");
    else $display("PASSED");
  end
endmodule
