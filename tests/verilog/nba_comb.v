// Combinational always @* blocks written with non-blocking assignments and defaults
// (`x <= 0; case ... x <= v;`), chained, as in RTL that uses this style.
module tb;
  reg [5:0] op;
  reg [1:0] f, g;
  reg [7:0] out;
  integer evals = 0;
  always @(*) begin
    f <= 2'b00;
    case (op[1:0])
      2'd1: f <= 2'b01;
      2'd2: f <= 2'b10;
      default: ;
    endcase
  end
  always @(*) begin
    evals = evals + 1;
    g <= 2'b11;
    out <= 8'h00;
    case (op[5:4])
      2'd0: begin g <= f; out <= {6'd0, f}; end
      2'd1: out <= 8'h55;
      default: ;
    endcase
  end
  initial begin
    op = 0;
    #1 $display("op=%h f=%b g=%b out=%h", op, f, g, out);
    op = 6'h01;
    #1 $display("op=%h f=%b g=%b out=%h", op, f, g, out);
    op = 6'h12;
    #1 $display("op=%h f=%b g=%b out=%h", op, f, g, out);
    op = 6'h02;
    #1 $display("op=%h f=%b g=%b out=%h evals>0=%b", op, f, g, out, evals > 0);
  end
endmodule
