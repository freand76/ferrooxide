// Parameters and function return types with several packed dimensions, and packed dimensions
// after an enum: the value is the whole vector, selects index the outer dimension first
// (IEEE 1800 6.20.2, 7.4.1, 7.4.5, 13.4.1).
package p;
  localparam bit [1:0][2:0][2:0] M = {{3'b011, 3'b101, 3'b111}, {3'b100, 3'b010, 3'b000}};
endpackage

module test;
  reg failed = 0;
  parameter [1:0][3:0] P = 8'h5a;
  localparam logic signed [1:0][7:0] S = 16'hff01;

  typedef enum logic [1:0] { A, B, C } e_t;
  enum logic [1:0] { X, Y, Z } [2:0] ev;
  e_t [1:0] ep;

  function automatic bit [3:0][7:0] swap(bit [3:0][7:0] x);
    for (int i = 0; i < 4; i++) swap[i] = x[3 - i];
  endfunction

  function logic [1:0][3:0] nib(input logic [7:0] v);
    nib = v;
  endfunction

  initial begin
    bit [3:0][7:0] w;
    if ($bits(p::M) != 18 || p::M[1][2] !== 3'b011 || p::M[0][0] !== 3'b000) failed = 1;
    if (p::M[1][0][2:1] !== 2'b11 || p::M[0][1] !== 3'b010) failed = 1;
    if (P !== 8'h5a || P[1] !== 4'h5 || P[0] !== 4'ha || $bits(P) != 8) failed = 1;
    if (S[1] !== 8'hff || S >= 0) failed = 1;
    w = swap(32'h01020304);
    if (w !== 32'h04030201 || w[0] !== 8'h01) failed = 1;
    if (nib(8'hc3) !== 8'hc3 || $bits(nib(0)) != 8) failed = 1;
    ev = '0;
    ev[0] = Y;
    ev[2] = Z;
    if ($bits(ev) != 6 || ev !== 6'b10_00_01 || ev[2] !== Z) failed = 1;
    ep = {C, B};
    if ($bits(ep) != 4 || ep[1] !== C || ep[0] !== B) failed = 1;
    if (failed) $display("FAILED");
    else $display("PASSED");
  end
endmodule
