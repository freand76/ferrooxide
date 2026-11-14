// Generate constructs (IEEE 1800 27): loops with genvars, if and case selected by
// parameters, named blocks and hierarchical names into them, the implicit genblk names
module adder #(parameter K = 0) (input [7:0] i, output [7:0] o);
  assign o = i + K;
endmodule
module t;
  integer errors = 0;
  parameter MODE = 2;
  wire [7:0] chain [0:4];
  assign chain[0] = 8'd1;
  genvar g;
  for (g = 0; g < 4; g = g + 1) begin : stage
    adder #(.K(g)) c(chain[g], chain[g + 1]);
    wire [7:0] tap = chain[g + 1];
  end
  generate
    if (MODE == 1) begin : sel
      wire [3:0] v = 1;
    end else if (MODE == 2) begin : sel
      wire [3:0] v = 2;
    end else begin : sel
      wire [3:0] v = 3;
    end
    case (MODE)
      1: begin : cs wire [7:0] v = 10; end
      2, 3: begin : cs wire [7:0] v = 20; end
      default: begin : cs wire [7:0] v = 30; end
    endcase
  endgenerate
  // nested loops
  for (g = 0; g < 2; g++) begin : outer
    genvar h;
    for (h = 0; h < 3; h++) begin : inner
      wire [7:0] id = g * 10 + h;
    end
  end
  // an unnamed generate if: genblk5, the fifth generate construct here (IEEE 1800 27.6)
  if (1) begin
    wire [3:0] anon = 4'd9;
  end
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1;
    check(`__LINE__, chain[4], 1 + 0 + 1 + 2 + 3);
    check(`__LINE__, stage[2].tap, 1 + 0 + 1 + 2);
    check(`__LINE__, stage[3].c.K, 3);
    check(`__LINE__, sel.v, 2);
    check(`__LINE__, cs.v, 20);
    check(`__LINE__, outer[1].inner[2].id, 12);
    check(`__LINE__, genblk5.anon, 9);
    if (errors == 0) $display("PASSED");
  end
endmodule
