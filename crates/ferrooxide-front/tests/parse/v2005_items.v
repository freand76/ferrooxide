`timescale 1ns/1ps
module m #(parameter integer N = 4, parameter [7:0] P = 8'hA5) (a, b, y, q);
  input [N-1:0] a, b;
  output y;
  output reg [N-1:0] q;
  wire (weak1, strong0) #3 w = a[0];
  wand wa;
  tri1 [1:0] t;
  localparam real R = 1.5e3;
  integer i;
  reg [7:0] mem [0:15];
  assign #(1, 2) y = &a | ^b;
  and #2 g1 (wa, a[0], b[0]);
  buf bb[1:0] (t, a[1:0]);
  sub #(.W(N)) u1 (.x(a), .y(), .z(q[0]));
  sub u2 [1:0] (a, , b);
  defparam u1.W = 3;
  specify
    (a => y) = (1, 2);
  endspecify
  genvar g;
  generate
    for (g = 0; g < N; g = g + 1) begin : gl
      wire x = a[g] ^ b[g];
    end
    if (N > 2) begin : big
      assign t[0] = 1'bz;
    end else begin
      assign t[1] = 1'b0;
    end
    case (N)
      1, 2: begin : small end
      default: ;
    endcase
  endgenerate
  function [3:0] f;
    input [3:0] x;
    f = x + 4'sd1;
  endfunction
  task automatic t2(input integer n, output reg [3:0] r);
    begin : body
      r = n;
      disable body;
    end
  endtask
endmodule

primitive mux (o, s, a, b);
  output o;
  input s, a, b;
  table
    0 1 ? : 1;
    1 ? 0 : 0;
  endtable
endprimitive

primitive lat (q, d, en);
  output reg q;
  input d, en;
  initial q = 1'b0;
  table
    ? 1 : ? : -;
    (01) 0 : ? : 1;
  endtable
endprimitive
