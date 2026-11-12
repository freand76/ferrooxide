// Synthetic RTL benchmark: N parallel lanes of LFSR + accumulator + small ALU, with a mux tree,
// clocked for CYCLES cycles. Mixes sequential (NBA) and combinational (assign / always @*) logic.
module lane #(parameter SEED = 1) (input clk, input rst, output reg [31:0] acc);
  reg [31:0] lfsr;
  wire fb = lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0];
  reg [31:0] alu;
  wire [1:0] op = lfsr[3:2];
  always @* begin
    case (op)
      2'd0: alu = acc + lfsr;
      2'd1: alu = acc ^ (lfsr << 3);
      2'd2: alu = acc - (lfsr >> 2);
      default: alu = {acc[15:0], acc[31:16]} & lfsr;
    endcase
  end
  always @(posedge clk) begin
    if (rst) begin
      lfsr <= SEED;
      acc <= 0;
    end else begin
      lfsr <= {lfsr[30:0], fb};
      acc <= alu;
    end
  end
endmodule

module tb;
  parameter N = 32;
  parameter CYCLES = 200000;
  reg clk = 0, rst = 1;
  wire [31:0] acc [0:N-1];
  genvar g;
  generate for (g = 0; g < N; g = g + 1) begin : L
    lane #(.SEED(g * 7919 + 1)) u(.clk(clk), .rst(rst), .acc(acc[g]));
  end endgenerate
  reg [31:0] x;
  integer i, c;
  initial begin
    #1 clk = 1; #1 clk = 0; rst = 0;
    for (c = 0; c < CYCLES; c = c + 1) begin
      #1 clk = 1; #1 clk = 0;
    end
    x = 0;
    for (i = 0; i < N; i = i + 1) x = x ^ acc[i];
    $display("checksum %h", x);
    $finish;
  end
endmodule
