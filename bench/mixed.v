// Second benchmark: deep combinational chain, 128-bit datapath (generic path), a memory with
// dynamic indexing, case-based FSM, and a testbench-driven clock.
module stage(input [31:0] a, input [31:0] k, output [31:0] y);
  assign y = ((a ^ k) + {a[7:0], a[31:8]}) ^ (a >> 3);
endmodule

module dut(input clk, input [31:0] din, output reg [127:0] wide, output reg [31:0] out);
  wire [31:0] s [0:8];
  assign s[0] = din;
  genvar i;
  generate for (i = 0; i < 8; i = i + 1) begin : ch
    stage st(.a(s[i]), .k(32'h9e3779b9 * (i + 1)), .y(s[i+1]));
  end endgenerate
  reg [31:0] mem [0:63];
  reg [5:0] wp, rp;
  reg [1:0] state;
  always @(posedge clk) begin
    mem[wp] <= s[8];
    wp <= wp + 1;
    rp <= wp - 5;
    wide <= {wide[95:0], mem[rp]} + {96'd0, s[8]};
    case (state)
      2'd0: begin out <= out + mem[rp]; state <= 2'd1; end
      2'd1: begin out <= out ^ s[8]; state <= out[0] ? 2'd2 : 2'd0; end
      2'd2: begin out <= {out[30:0], out[31]}; state <= 2'd3; end
      default: begin out <= out - wide[31:0]; state <= 2'd0; end
    endcase
  end
  integer k;
  initial begin
    wp = 0; rp = 0; state = 0; out = 0; wide = 0;
    for (k = 0; k < 64; k = k + 1) mem[k] = k * 17;
  end
endmodule

module tb;
  reg clk = 0;
  reg [31:0] din = 1;
  wire [127:0] wide;
  wire [31:0] out;
  dut d(.clk(clk), .din(din), .wide(wide), .out(out));
  integer c;
  initial begin
    for (c = 0; c < 300000; c = c + 1) begin
      #1 clk = 1;
      din = din * 1103515245 + 12345;
      #1 clk = 0;
    end
    $display("out=%h wide=%h", out, wide);
    $finish;
  end
endmodule
