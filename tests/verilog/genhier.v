// Hierarchical names with genvar-computed indices: reads, writes and defparam
module leaf;
  parameter P = 0;
  reg [3:0] v;
endmodule

module top;
  genvar i;
  generate
    for (i = 1; i < 3; i = i + 1) begin : U
      reg [1:0] x;
      leaf m ();
    end
    for (i = 0; i < 2; i = i + 1) begin : V
      initial begin
        U[(i + 1) % 4].x = i + 1;
        U[i + 1].m.v = 4'd9 - i;
      end
      defparam U[i + 1].m.P = 10 * (i + 1);
    end
  endgenerate
  initial #1 $display("%0d %0d %0d %0d %0d %0d", U[1].x, U[2].x, U[1].m.v, U[2].m.v, U[1].m.P, U[2].m.P);
endmodule
